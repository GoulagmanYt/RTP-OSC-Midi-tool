$ErrorActionPreference = 'Stop'

# Load only the helper under test; never execute the build pipeline here.
$buildScript = Join-Path $PSScriptRoot 'build-windows.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($buildScript, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw $parseErrors[0] }
$helper = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Stop-WorkspaceNodeProcesses' }, $true)
if (-not $helper) { throw 'Missing workspace Node lock cleanup helper.' }
. ([scriptblock]::Create($helper.Extent.Text))

$repositoryRoot = 'C:\Build workspace\OSCMidi'
$script:stopped = @()
$script:messages = @()
function Write-BuildLog { param($Message, $Color) $script:messages += $Message }
function Stop-Process { param($InputObject, [switch]$Force, $ErrorAction) $script:stopped += $InputObject.Id }

function New-TestProcess {
    param([int]$ProcessId, [string[]]$Modules)
    $process = [PSCustomObject]@{ Id = $ProcessId; Modules = @($Modules | ForEach-Object { [PSCustomObject]@{ FileName = $_ } }) }
    $process | Add-Member ScriptMethod WaitForExit { param($Timeout) return $true }
    return $process
}

$script:candidates = @(
    (New-TestProcess 101 @('C:\Build workspace\OSCMidi\node_modules\lightningcss\binding.node'))
    (New-TestProcess 102 @('C:\Build workspace\OSCMidi-other\node_modules\binding.node'))
    (New-TestProcess 103 @('C:\Other app\node_modules\binding.node'))
    (New-TestProcess 104 @('C:\Build workspace\OSCMidi\node_modules-old\binding.node'))
    (New-TestProcess 105 @('C:\Build workspace\OSCMidi\node_modules\package\readme.txt'))
    (New-TestProcess 106 @('c:\build WORKSPACE\oscmidi\node_modules\@tailwindcss\binding.node'))
)
function Get-Process { param($Name, $ErrorAction) return $script:candidates }

Stop-WorkspaceNodeProcesses
if (($script:stopped -join ',') -ne '101,106') { throw "Wrong processes stopped: $($script:stopped -join ',')" }
if ($script:messages.Count -ne 2) { throw 'Every stopped process must be logged.' }

# An uninspectable process must not cause unrelated processes to be terminated.
$inaccessible = [PSCustomObject]@{ Id = 107 }
$inaccessible | Add-Member ScriptProperty Modules { throw 'Access denied' }
$script:candidates = @($inaccessible, (New-TestProcess 108 @()))
$script:stopped = @()
Stop-WorkspaceNodeProcesses
if ($script:stopped.Count) { throw 'Stopped a process without evidence of a workspace lock.' }

Write-Host 'PASS: workspace native module holders are stopped; unrelated, sibling, and uninspectable processes are preserved.'
