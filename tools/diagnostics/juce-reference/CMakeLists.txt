cmake_minimum_required(VERSION 3.22)
project(OSCMidiReferenceHost VERSION 1.0 LANGUAGES C CXX)
if(NOT DEFINED JUCE_SOURCE_DIR)
  message(FATAL_ERROR "Set JUCE_SOURCE_DIR to a local JUCE 8.0.15 checkout")
endif()
add_subdirectory("${JUCE_SOURCE_DIR}" juce EXCLUDE_FROM_ALL)
juce_add_console_app(juce_reference PRODUCT_NAME "Independent VST3 reference")
target_sources(juce_reference PRIVATE main.cpp)
target_compile_features(juce_reference PRIVATE cxx_std_17)
target_compile_definitions(juce_reference PRIVATE JUCE_PLUGINHOST_VST3=1 JUCE_ASIO=1
  JUCE_WEB_BROWSER=0 JUCE_USE_CURL=0 JUCE_MODAL_LOOPS_PERMITTED=1)
target_link_libraries(juce_reference PRIVATE juce::juce_audio_utils psapi
  juce::juce_recommended_config_flags)
