# xcode-select --install
# brew install qt opencv p7zip

export PATH=/usr/local/Cellar/qt/6.2.0/bin:$PATH
export FFMPEG_DIR=/users/Admin/gyroflow/ext/ffmpeg-4.4-macOS-lite-x86_64/

export OPENCV_LINK_LIBS="opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video"
export OPENCV_LINK_PATHS=/usr/local/Cellar/opencv/4.5.3_2/lib
export OPENCV_INCLUDE_PATHS=/usr/local/Cellar/opencv/4.5.3_2/include/opencv4
export PATH=/usr/local/Cellar/opencv/4.5.3_2/bin/:$PATH
#export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/usr/lib/"
export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/Toolchains/XcodeDefault.xctoolchain/usr/lib/"


# Rust cannot link to fat libraries, so extract the architectures to a single files
# lipo libavcodec.a    -thin x86_64 -output libavcodec-x86_64.a     ;  mv libavcodec-x86_64.a    libavcodec.a
# lipo libavformat.a   -thin x86_64 -output libavformat-x86_64.a    ;  mv libavformat-x86_64.a   libavformat.a
# lipo libavdevice.a   -thin x86_64 -output libav-device-x86_64.a   ;  mv libav-device-x86_64.a  libavdevice.a
# lipo libavfilter.a   -thin x86_64 -output libavfilter-x86_64.a    ;  mv libavfilter-x86_64.a   libavfilter.a
# lipo libavutil.a     -thin x86_64 -output libavutil-x86_64.a      ;  mv libavutil-x86_64.a     libavutil.a
# lipo libswresample.a -thin x86_64 -output libswresample-x86_64.a  ;  mv libswresample-x86_64.a libswresample.a
# lipo libswscale.a    -thin x86_64 -output libswscale-x86_64.a     ;  mv libswscale-x86_64.a    libswscale.a
