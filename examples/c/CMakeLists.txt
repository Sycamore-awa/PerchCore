cmake_minimum_required(VERSION 3.16)
project(perch_core_example C)

set(PERCH_CORE_LIBRARY "" CACHE FILEPATH "Absolute path to the built PerchCore shared library")
if(NOT EXISTS "${PERCH_CORE_LIBRARY}")
    message(FATAL_ERROR "Set PERCH_CORE_LIBRARY to the built libperch_ffi.so/dylib or import library")
endif()

add_executable(perch-example main.c)
target_compile_features(perch-example PRIVATE c_std_99)
target_include_directories(perch-example PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../../include")
target_link_libraries(perch-example PRIVATE "${PERCH_CORE_LIBRARY}")

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(perch-example PRIVATE -Wall -Wextra -Werror)
endif()

enable_testing()
add_test(NAME c_abi_version COMMAND perch-example)
set_tests_properties(c_abi_version PROPERTIES PASS_REGULAR_EXPRESSION "\"core\":\"0\\.1\\.0\"")
add_test(NAME c_abi_error COMMAND perch-example unknown.method)
set_tests_properties(c_abi_error PROPERTIES PASS_REGULAR_EXPRESSION "FFI_UNKNOWN_METHOD")
add_test(NAME c_abi_unicode_locator COMMAND perch-example locator.validate "{\"href\":\"中文.txt\",\"type\":\"text/plain\",\"perch\":{\"v\":1,\"format\":\"txt\",\"txtOffset\":0}}")
set_tests_properties(c_abi_unicode_locator PROPERTIES PASS_REGULAR_EXPRESSION "\"valid\":true")
