$ErrorActionPreference = 'Stop'
foreach ($file in @('build-windows.ps1', 'build-menu.ps1', 'build-support.ps1')) {
    $tokens = $null; $parseErrors = $null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
}
. (Join-Path $PSScriptRoot 'build-support.ps1')
. (Join-Path $PSScriptRoot 'build-menu.ps1')

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Select-TestBuild([string[]]$Answers) {
    $script:answers = [Collections.Generic.Queue[string]]::new()
    foreach ($answer in $Answers) { $script:answers.Enqueue($answer) }
    return Read-BuildSelection -ReadInput {
        param($Prompt)
        if ($script:answers.Count -eq 0) { throw "Unexpected prompt: $Prompt" }
        return $script:answers.Dequeue()
    }
}

$selection = Select-TestBuild @('', '', '', '', '')
Assert-True ($selection.BuildMode -eq 'Dev') 'Enter must select the fast standalone Dev build.'
Assert-True ($selection.ValidationMode -eq 'Quick') 'Dev must default to quick checks without running tests.'
Assert-True (-not $selection.NoLog) 'Build logs must default to enabled.'
Assert-True ($selection.CleanupMode -eq 'None') 'Caches must be retained by default.'

$selection = Select-TestBuild @('invalid', '3', '0', '2', '3', '')
Assert-True ($selection.BuildMode -eq 'Both') 'Both profiles must be selectable after invalid input.'
Assert-True ($selection.ValidationMode -eq 'None') 'Checks must be optional independently of the build profile.'
Assert-True $selection.NoLog 'The no-log choice must reach the build pipeline.'
Assert-True ($selection.CleanupMode -eq 'Full') 'Full post-build cleanup must be selectable.'
Assert-True (-not $selection.FullClean) 'Full post-build cleanup must preserve exported deliverables.'

$selection = Select-TestBuild @('4', '4', 'RESET')
Assert-True $selection.FullClean 'The explicit reset operation must include old deliverables.'
Assert-True $selection.CleanOnly 'Reset must never start a build.'
Assert-True $selection.NoLog 'Reset must not recreate the deleted log directory.'

$selection = Select-TestBuild @('2', '2', '1', '1', 'R', '0')
Assert-True $selection.Cancelled 'Review must allow changing choices and cancelling without running anything.'

foreach ($mode in @('Dev', 'Release', 'Both')) {
    $plan = @(Get-WindowsBuildPlan -BuildMode $mode -CleanupMode Partial)
    $profiles = @($plan | Where-Object Kind -eq 'Desktop' | ForEach-Object Profile)
    $expected = switch ($mode) { Dev { 'debug' } Release { 'release' } Both { 'debug,release' } }
    Assert-True (($profiles -join ',') -eq $expected) "Wrong desktop profiles for $mode."
    Assert-True (@($plan | Where-Object Kind -eq 'Dependencies').Count -eq 1) 'Dependencies must be prepared only once.'
    Assert-True (@($plan | Where-Object Kind -eq 'Frontend').Count -eq 1) 'The frontend must be built only once.'
    Assert-True (@($plan | Where-Object Kind -eq 'ValidateRust').Count -eq $(if ($mode -eq 'Dev') { 0 } else { 1 })) 'Dev must skip the long Rust validation; Release must keep it.'
    Assert-True ($plan[-1].Kind -eq 'Cleanup') 'Cleanup must happen after every selected profile is exported.'
    foreach ($profile in $profiles) {
        $worker = @($plan | Where-Object { $_.Kind -eq 'Workers' -and $_.Profile -eq $profile })
        Assert-True ($worker.Count -eq 1) "Workers must use the $profile profile exactly once."
    }
}

foreach ($mode in @('Dev', 'Release', 'Both')) {
    foreach ($checks in @('None', 'Quick', 'Full')) {
        $plan = @(Get-WindowsBuildPlan -BuildMode $mode -ValidationMode $checks)
        $validationSteps = @($plan | Where-Object { $_.Kind -like 'Validate*' })
        $expected = switch ($checks) { None { '' } Quick { 'ValidateQuick' } Full { 'ValidateFrontend,ValidateRust' } }
        Assert-True (($validationSteps.Kind -join ',') -eq $expected) "$mode / $checks must run only the selected checks."
    }
}

$devArguments = @(Get-DesktopBuildArguments -Profile debug -ConfigPath 'C:\build path\config.json')
Assert-True ($devArguments -contains '--debug' -and $devArguments -contains '--no-bundle') 'Dev must compile debug without MSI packaging.'
Assert-True ($devArguments -contains 'C:\build path\config.json') 'Tauri config paths with spaces must remain one argument.'
$releaseArguments = @(Get-DesktopBuildArguments -Profile release -ConfigPath 'C:\config.json')
Assert-True ($releaseArguments -contains 'msi' -and $releaseArguments -notcontains '--debug') 'Release must retain optimized MSI packaging.'

Write-Host 'PASS: menu defaults, invalid input, profile/log/cleanup selection, review/cancel, shared build steps and Tauri arguments.'
