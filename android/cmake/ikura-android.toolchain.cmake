# Stable settings for SDL 2.26.4's CMake build with recent Android NDKs.
set(ANDROID_ABI arm64-v8a CACHE STRING "Android ABI")
set(ANDROID_PLATFORM android-24 CACHE STRING "Android API level")

# This game does not use device sensors. SDL 2.26.4 calls the removed
# ALooper_pollAll API in its Android sensor backend, so omit that subsystem.
set(SDL_SENSOR OFF CACHE BOOL "Disable unused SDL sensor support")

include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")
