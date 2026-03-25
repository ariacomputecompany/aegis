cmake_minimum_required(VERSION 3.21)
project(aegis_native)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS ON)

find_package(Python3 REQUIRED COMPONENTS Interpreter)

set(CEF_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64")
set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} "${CEF_ROOT}/cmake")

find_package(CEF REQUIRED)

add_subdirectory(${CEF_LIBCEF_DLL_WRAPPER_PATH} libcef_dll_wrapper)

set(AEGIS_NATIVE_TARGET "aegis_native")
set(AEGIS_NATIVE_HELPER_TARGET "aegis_native_Helper")
set(AEGIS_NATIVE_HELPER_OUTPUT_NAME "aegis_native Helper")
set(AEGIS_HOST_LIBRARY_TARGET "aegis_host")
set(AEGIS_SHARED_SUPPORT_TARGET "aegis_shared_support")
set(AEGIS_GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")
set(AEGIS_RUNTIME_SCRIPT_JS "${CMAKE_CURRENT_SOURCE_DIR}/../assets/js/aegis_runtime.js")
set(AEGIS_RUNTIME_SCRIPT_HEADER "${AEGIS_GENERATED_DIR}/aegis_runtime_script.h")

add_custom_command(
  OUTPUT ${AEGIS_RUNTIME_SCRIPT_HEADER}
  COMMAND ${Python3_EXECUTABLE}
          ${CMAKE_CURRENT_SOURCE_DIR}/../scripts/generate_runtime_header.py
          ${AEGIS_RUNTIME_SCRIPT_JS}
          ${AEGIS_RUNTIME_SCRIPT_HEADER}
  DEPENDS
    ${AEGIS_RUNTIME_SCRIPT_JS}
    ${CMAKE_CURRENT_SOURCE_DIR}/../scripts/generate_runtime_header.py
  VERBATIM
)

add_custom_target(aegis_runtime_header DEPENDS ${AEGIS_RUNTIME_SCRIPT_HEADER})

SET_CEF_TARGET_OUT_DIR()

set(AEGIS_NATIVE_SOURCES
  ${AEGIS_RUNTIME_SCRIPT_HEADER}
  aegis_app.h
  aegis_app.cc
  aegis_client.h
  aegis_client.cc
  aegis_state_paths.h
  aegis_state_paths.cc
  aegis_message_pump_mac.mm
  aegis_browser_host_mac.mm
  aegis_native_mac.mm
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
)

set(AEGIS_NATIVE_HELPER_SOURCES
  process_helper_mac.cc
  aegis_app.cc
  aegis_client.cc
  aegis_message_pump_mac.mm
)

set(AEGIS_HOST_LIBRARY_SOURCES
  aegis_state_paths.cc
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
)

set_source_files_properties(
  src/aegis_protocol.cpp
  src/aegis_cef_host.cpp
  PROPERTIES COMPILE_FLAGS "-fexceptions"
)

set(AEGIS_NATIVE_RESOURCES
  mac/Info.plist.in
  mac/helper-Info.plist.in
  mac/English.lproj/InfoPlist.strings
)

set(CEF_APP "${CEF_TARGET_OUT_DIR}/${AEGIS_NATIVE_TARGET}.app")
set(EXECUTABLE_NAME "${AEGIS_NATIVE_TARGET}")
set(PRODUCT_NAME "${AEGIS_NATIVE_TARGET}")

add_executable(${AEGIS_NATIVE_TARGET} MACOSX_BUNDLE ${AEGIS_NATIVE_SOURCES} ${AEGIS_NATIVE_RESOURCES})
add_dependencies(${AEGIS_NATIVE_TARGET} aegis_runtime_header)
SET_EXECUTABLE_TARGET_PROPERTIES(${AEGIS_NATIVE_TARGET})
target_include_directories(${AEGIS_NATIVE_TARGET} PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}
  ${AEGIS_GENERATED_DIR}
  ${CMAKE_CURRENT_SOURCE_DIR}/include
  ${CEF_ROOT}
  ${CEF_ROOT}/include
)
target_compile_options(${AEGIS_NATIVE_TARGET} PRIVATE -fexceptions)
target_compile_definitions(${AEGIS_NATIVE_TARGET} PRIVATE AEGIS_STANDALONE_APP=1)
target_link_libraries(${AEGIS_NATIVE_TARGET} libcef_dll_wrapper ${CEF_STANDARD_LIBS} "-framework QuartzCore")
set_target_properties(${AEGIS_NATIVE_TARGET} PROPERTIES
  MACOSX_BUNDLE_INFO_PLIST ${CMAKE_CURRENT_SOURCE_DIR}/mac/Info.plist.in
  CLANG_ENABLE_OBJC_ARC YES
  XCODE_ATTRIBUTE_GCC_ENABLE_CPP_EXCEPTIONS YES
  XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "com.ariacomputecompany.aegis"
)

add_library(${AEGIS_SHARED_SUPPORT_TARGET} STATIC
  ${AEGIS_RUNTIME_SCRIPT_HEADER}
  aegis_app.cc
  aegis_client.cc
  aegis_state_paths.cc
  aegis_message_pump_mac.mm
  aegis_browser_host_mac.mm
)
add_dependencies(${AEGIS_SHARED_SUPPORT_TARGET} aegis_runtime_header)
target_compile_features(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE cxx_std_20)
target_include_directories(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}
  ${AEGIS_GENERATED_DIR}
  ${CMAKE_CURRENT_SOURCE_DIR}/include
  ${CEF_ROOT}
  ${CEF_ROOT}/include
)
target_link_libraries(${AEGIS_SHARED_SUPPORT_TARGET} PRIVATE
  libcef_dll_wrapper
  ${CEF_STANDARD_LIBS}
  "-framework QuartzCore"
)

add_library(${AEGIS_HOST_LIBRARY_TARGET} SHARED ${AEGIS_HOST_LIBRARY_SOURCES})
target_include_directories(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}
  ${CMAKE_CURRENT_SOURCE_DIR}/include
  ${CEF_ROOT}
  ${CEF_ROOT}/include
)
target_compile_features(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE cxx_std_20)
target_link_libraries(${AEGIS_HOST_LIBRARY_TARGET} PRIVATE
  ${AEGIS_SHARED_SUPPORT_TARGET}
  libcef_dll_wrapper
  ${CEF_STANDARD_LIBS}
)
set_target_properties(${AEGIS_HOST_LIBRARY_TARGET} PROPERTIES
  LIBRARY_OUTPUT_DIRECTORY "${CEF_TARGET_OUT_DIR}"
  OUTPUT_NAME "aegis_host"
  XCODE_ATTRIBUTE_CODE_SIGNING_ALLOWED "NO"
  XCODE_ATTRIBUTE_CODE_SIGNING_REQUIRED "NO"
)

add_dependencies(${AEGIS_NATIVE_TARGET} ${AEGIS_HOST_LIBRARY_TARGET})
add_custom_command(
  TARGET ${AEGIS_NATIVE_TARGET}
  POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
          "${CEF_TARGET_OUT_DIR}/libaegis_host.dylib"
          "${CEF_APP}/Contents/Frameworks/libaegis_host.dylib"
  VERBATIM
)

COPY_MAC_FRAMEWORK("${AEGIS_NATIVE_TARGET}" "${CEF_BINARY_DIR}" "${CEF_APP}")

foreach(_suffix_list ${CEF_HELPER_APP_SUFFIXES})
  string(REPLACE ":" ";" _suffix_list ${_suffix_list})
  list(GET _suffix_list 0 _name_suffix)
  list(GET _suffix_list 1 _target_suffix)
  list(GET _suffix_list 2 _plist_suffix)

  set(_helper_target "${AEGIS_NATIVE_HELPER_TARGET}${_target_suffix}")
  set(_helper_output_name "${AEGIS_NATIVE_HELPER_OUTPUT_NAME}${_name_suffix}")
  set(_helper_info_plist "${CMAKE_CURRENT_BINARY_DIR}/helper-Info${_target_suffix}.plist")

  file(READ "${CMAKE_CURRENT_SOURCE_DIR}/mac/helper-Info.plist.in" _plist_contents)
  string(REPLACE "\${EXECUTABLE_NAME}" "${_helper_output_name}" _plist_contents ${_plist_contents})
  string(REPLACE "\${PRODUCT_NAME}" "${_helper_output_name}" _plist_contents ${_plist_contents})
  string(REPLACE "\${BUNDLE_ID_SUFFIX}" "${_plist_suffix}" _plist_contents ${_plist_contents})
  file(WRITE ${_helper_info_plist} ${_plist_contents})

  add_executable(${_helper_target} MACOSX_BUNDLE ${AEGIS_NATIVE_HELPER_SOURCES})
  add_dependencies(${_helper_target} aegis_runtime_header)
  SET_EXECUTABLE_TARGET_PROPERTIES(${_helper_target})
  target_include_directories(${_helper_target} PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${AEGIS_GENERATED_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${CEF_ROOT}
    ${CEF_ROOT}/include
  )
  target_link_libraries(${_helper_target} libcef_dll_wrapper ${CEF_STANDARD_LIBS})
  set_target_properties(${_helper_target} PROPERTIES
    MACOSX_BUNDLE_INFO_PLIST ${_helper_info_plist}
    OUTPUT_NAME ${_helper_output_name}
    CLANG_ENABLE_OBJC_ARC YES
    XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "com.ariacomputecompany.aegis.helper${_plist_suffix}"
  )

  add_dependencies(${AEGIS_NATIVE_TARGET} "${_helper_target}")
  add_custom_command(
    TARGET ${AEGIS_NATIVE_TARGET}
    POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_directory
            "${CEF_TARGET_OUT_DIR}/${_helper_output_name}.app"
            "${CEF_APP}/Contents/Frameworks/${_helper_output_name}.app"
    VERBATIM
  )
endforeach()

set(PREFIXES "mac/")
COPY_MAC_RESOURCES("${AEGIS_NATIVE_RESOURCES}" "${PREFIXES}" "${AEGIS_NATIVE_TARGET}" "${CMAKE_CURRENT_SOURCE_DIR}" "${CEF_APP}")
