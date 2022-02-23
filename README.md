<p align="center">
  <h1 align="center">
    <a href="https://github.com/gyroflow/gyroflow#gh-light-mode-only">
      <img src="./resources/logo_black.svg" alt="Gyroflow logo" height="100">
    </a>
    <a href="https://github.com/gyroflow/gyroflow#gh-dark-mode-only">
      <img src="./resources/logo_white.svg" alt="Gyroflow logo" height="100">
    </a>
  </h1>

  <p align="center">
    Video stabilization using gyroscope data
    <br/>
    <br/>
    <a href="https://gyroflow.xyz">Homepage</a> •
    <a href="https://github.com/gyroflow/gyroflow/releases">Download</a> •
    <a href="https://docs.gyroflow.xyz">Documentation</a> •
    <a href="https://discord.gg/WfxZZXjpke">Discord</a> •
    <a href="https://github.com/gyroflow/gyroflow/issues">Report bug</a> •
    <a href="https://github.com/gyroflow/gyroflow/issues">Request feature</a>
  </p>
  <p align="center">
    <a href="https://github.com/gyroflow/gyroflow/releases">
      <img src="https://img.shields.io/github/downloads/gyroflow/gyroflow/total" alt="Downloads">
    </a>
    <a href="https://github.com/gyroflow/gyroflow/">
      <img src="https://img.shields.io/github/contributors/gyroflow/gyroflow?color=dark-green" alt="Contributors">
    </a>
    <a title="Crowdin" target="_blank" href="https://crowdin.com/project/gyroflow"><img src="https://badges.crowdin.net/gyroflow/localized.svg">
    </a>
    <a href="https://github.com/gyroflow/gyroflow/issues/">
      <img src="https://img.shields.io/github/issues/gyroflow/gyroflow" alt="Issues">
    </a>
    <a href="https://github.com/gyroflow/gyroflow/blob/master/LICENSE">
      <img src="https://img.shields.io/github/license/gyroflow/gyroflow" alt="License">
    </a>
  </p>
</p>

## About the project
Gyroflow is an application that can stabilize your video by using motion data from a gyroscope and optionally an accelerometer. Modern cameras record that data internally (GoPro, Sony, Insta360 etc), and this application stabilizes the captured footage precisely by using them. It can also use gyro data from an external source (eg. from Betaflight blackbox).

[Trailer / results video](https://www.youtube.com/watch?v=QR-SINyvNyI)

![Screenshot](resources/screenshot.jpg)

<p align="center">
  <a href="resources/comparison1.mp4"><img src="resources/comparison1.gif" height="200"></a>
  <a href="resources/comparison2.mp4"><img src="resources/comparison2.gif" height="200"></a>
</p>

## Features
- Real time preview, params adjustments and all calculations
- GPU processing and rendering
- Fully multi-threaded
- Rolling shutter correction
- Supports already stabilized GoPro videos (captured with Hypersmooth enabled) (Hero 8 and up)
- Supports and renders 10-bit videos (and higher, up to 16-bit 4:4:4, direct YUV rendering with no data loss - no conversion to RGB)
- Visual chart with gyro data (can display gyro, accel, magnetometer and quaternions)
- Visual display of smoothed quaternions
- Real time offset adjustments
- Two optical flow methods
- Two offsets calculation methods
- Modern responsive user interface with Dark and Light theme
- Adaptive zoom (dynamic cropping)
- Based on [telemetry-parser](https://github.com/AdrianEddy/telemetry-parser) - supports all gyro sources out of the box
- Gyro low pass filter, arbitrary rotation (pitch, roll, yaw angles) and orientation
- Multiple gyro integration methods for orientation determination
- Multiple video orientation smoothing algorithms, including horizon levelling and per-axis smoothness adjustment.
- Cross-platform - currently works on Windows/Linux/Mac, with Android and iOS apps coming
- Multiple UI languages
- Supports variable and high frame rate videos, all calculations are done on timestamps
- x264, x265, ProRes and PNG outputs, with x264 and x265 fully GPU accelerated
- Automatic lens calibration process
- Fully zero-copy GPU preview rendering is possible, implemented and *almost* working (😀)
- Core engine is a separate library without external dependencies (no Qt, no ffmpeg, no OpenCV), and can be used to create OpenFX and Adobe plugins (on the TODO list)
- Automatic updates of lens profile database
- Built-in lens profiles for GoPro HERO 6, 8, 9 and 10 in all shooting modes

## Supported gyro sources
- [x] GoPro (HERO 5 and later)
- [x] Sony (a1, a7c, a7r IV, a7 IV, a7s III, a9 II, FX3, FX6, RX0 II, RX100 VII, ZV1, ZV-E10)
- [x] Insta360 (OneR, SMO 4k, Go, GO2)
- [x] Betaflight blackbox (CSV and binary)
- [x] Runcam CSV (Runcam 5 Orange, iFlight GOCam GR, Runcam Thumb, Mobius Maxi 4K)
- [x] Hawkeye Firefly X Lite CSV
- [x] WitMotion (WT901SDCL binary and *.txt)
- [x] iOS apps: `Sensor Logger`, [`G-Field Recorder`](https://apps.apple.com/at/app/g-field-recorder/id1154585693), [`Gyro`](https://apps.apple.com/us/app/gyro-record-device-motion-data/id1161532981)
- [x] Android apps: [`Sensor Logger`](https://play.google.com/store/apps/details?id=com.kelvin.sensorapp&hl=de_AT&gl=US), [`Sensor Record`](https://play.google.com/store/apps/details?id=de.martingolpashin.sensor_record)
- [x] Gyroflow [.gcsv log](https://docs.gyroflow.xyz/logging/gcsv/)
- [x] ArduPilot VideoStabilization logging (*.log)

### Info for cameras not on the list

- For cameras which do have built-in gyro, please contact us and we will implement support for that camera. Refer to the [documentation](https://docs.gyroflow.xyz) for information about the gyro logging process.
- For cameras which don't have built-in gyro, please consider using Betaflight FC or check out our [flowshutter](https://github.com/gyroflow/flowshutter) project.

## Minimum system requirements:
- Windows 10 64-bit (1809 or later) [Install VC redist if it doesn't run](https://aka.ms/vs/17/release/vc_redist.x64.exe)
    - If you have Windows "N" install, go to `Settings` -> `Apps` -> `Optional features` -> `Add a feature` -> enable `Media Feature Pack`
- macOS 10.14 or later (both Intel and Apple Silicon are supported natively)
- Linux: 
    - `.tar.gz` package (recommended): Debian 10+, Ubuntu 18.10+, CentOS 8.2+, openSUSE 15.3+. Other distros require glibc 2.28+ (`ldd --version` to check)
    - `.AppImage` should work everywhere
    - Make sure you have latest graphics drivers installed
    - Possibly needed packages: `sudo apt install libva2 libvdpau1 libasound2 libxkbcommon0 libpulse0 libc++-dev vdpau-va-driver libvulkan1`
    - GPU specific packages: 
        - NVIDIA: `nvidia-opencl-icd nvidia-vdpau-driver nvidia-egl-icd nvidia-vulkan-icd libnvcuvid1 libnvidia-encode1`
        - Intel: `intel-media-va-driver i965-va-driver beignet-opencl-icd intel-opencl-icd`
        - AMD: `mesa-vdpau-drivers mesa-va-drivers mesa-opencl-icd libegl-mesa0 mesa-vulkan-drivers`
- Android 6+

## Help and support
For general support and discussion, you can find the developers and other users on the [Gyroflow Discord server](https://discord.gg/BBJ2UVAr2D). 

For companies or people wishing to get in touch with the team privately for collaborative purposes: devteam@gyroflow.xyz.

## Roadmap

See the [open issues](https://github.com/gyroflow/gyroflow/issues) for a list of proposed features and known issues.
There's also a ton of TODO comments throughout the code.

### Video editor plugins 
Gyroflow OpenFX plugin is available [here](https://github.com/gyroflow/gyroflow-ofx).

Adobe After Effects and Davinci Resolve plugins are planned, but not ready yet

## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributors are **greatly appreciated**.
* If you have suggestions for adding or removing features, feel free to [open an issue](https://github.com/gyroflow/gyroflow/issues/new) to discuss it.
* If you want to implement a feature, you can fork this project, implement your code and open a pull request.

### Translations
Currently *Gyroflow* is available in:
* **English** (base language)
* **Chinese Simplified** (by [DusKing1](https://github.com/DusKing1))
* **Chinese Traditional** (by [DusKing1](https://github.com/DusKing1))
* **Danish** (by [ElvinC](https://github.com/ElvinC))
* **Finnish** (by Jesse Julkunen)
* **French** (by KennyDorion)
* **Galician** (by Martín Costas)
* **German** (by [Grommi](https://github.com/Gro2mi) and [Nicecrash](https://github.com/B-nutze-RR))
* **Greek** (by Stamatis Galiatsatos)
* **Indonesian** (by Aloysius Puspandono)
* **Italian** (by Rosario Casciello)
* **Japanese** (by 井上康)
* **Norwegian** (by [MiniGod](https://github.com/MiniGod) and [alexagv](https://github.com/alexagv))
* **Polish** (by [AdrianEddy](https://github.com/AdrianEddy))
* **Portuguese Brazilian** (by KallelGaNewk)
* **Portuguese** (by Ricardo Pimentel)
* **Russian** (by Андрей Гурьянов, redstar01 and lukdut)
* **Slovak** (by Radovan Leitman)
* **Spanish** (by Pelado-Mat)
* **Ukrainian** (by Artem Alexandrov)

Help us translate *Gyroflow* to your language! We use *crowdin* to manage translations and you can contribute there: https://crowdin.com/project/gyroflow

#### I want to contribute but I don't know Rust or QML
* The Rust book is a great way to get started with Rust: https://doc.rust-lang.org/book/
* Additional useful resources for Rust: https://quickref.me/rust and https://cheats.rs/
* For the UI stuff, there's a nice QML book by The Qt Company: https://www.qt.io/product/qt6/qml-book


## Development
### Used languages and technologies
*Gyroflow* is written in [Rust](https://www.rust-lang.org/), with UI written in [QML](https://doc.qt.io/qt-6/qmlfirststeps.html). It uses *Qt*, *ffmpeg*, *OpenCV* and *mdk-sdk* external dependencies for the main program, but the core library is written in pure Rust without any external dependencies.

OpenCV usage is kept to a minimum, used only for lens calibration and optical flow (`src/core/calibration/mod.rs` and `src/core/synchronization/opencv.rs`). Core algorithms and undistortion don't use OpenCV.

GPU stuff supports *DirectX*, *OpenGL*, *Metal* and *Vulkan* thanks to *Qt RHI* and *wgpu*.
For GPU processing we use *OpenCL* or *wgpu*, with highly parallelized CPU implementation as a fallback.

### Code structure
1. Entire GUI is in the `src/ui` directory
2. `src/controller.rs` is a bridge between UI and core, it takes all commands from QML and calls functions in core
3. `src/core` contains the whole gyroflow engine and doesn't depend on *Qt* or *ffmpeg*, and *OpenCV* is optional
4. `src/rendering` contains all FFmpeg related code for rendering final video and processing for synchronization
5. `src/core/gpu` contains GPU implementations of the undistortion
6. `src/qt_gpu` contains zero-copy GPU undistortion path, using Qt RHI and GLSL compute shader, but this method is experimental and buggy for now
7. `src/gyroflow.rs` is the main entry point
8. `mod.rs` or `lib.rs` in each directory act as a main entry of the module (directory name is the module name and `mod.rs` is kind of an entry point)

### Dev environment
`Visual Studio Code` with `rust-analyzer` extension.

For working with QML I recommend to use Qt Creator and load all QML files there, as it has auto-complete and syntax highlighting.
The project also supports UI live reload, it's a super quick way of working with the UI. Just change `live_reload = true` in `gyroflow.rs` and it should work right away. Now every time you change any QML file, the app should reload it immediately.

### Building on Windows
1. Get latest stable Rust language from: https://rustup.rs/
    - Please make sure to check the English language pack option when installing the C++ build tools from Visual Studio Installer 
2. Clone the repo: `git clone https://github.com/gyroflow/gyroflow.git`
3. Install dependencies to the `ext` directory: `cd gyroflow/ext`
    - `Qt 6.2.3` or higher: `pip3 install -U pip & pip3 install aqtinstall` then `aqt install-qt windows desktop 6.2.3 win64_msvc2019_64` or use the [official installer](https://www.qt.io/download-qt-installer)
    - `FFmpeg 5.0`: download [ffmpeg 5.0 lite](https://sourceforge.net/projects/avbuild/files/windows-desktop/ffmpeg-5.0-windows-desktop-clang-gpl-lite.tar.xz/download) and unzip it to `ext/ffmpeg-5.0-windows-desktop-clang-gpl-lite`
    - vcpkg: `git clone --depth 1 https://github.com/Microsoft/vcpkg.git & .\vcpkg\bootstrap-vcpkg.bat -disableMetrics`
    - OpenCV: `.\vcpkg\vcpkg install "opencv[core]:x64-windows-release"`
    - OpenCL: `.\vcpkg\vcpkg install "opencl:x64-windows-release"`
    - LLVM: download and install [LLVM](https://github.com/llvm/llvm-project/releases/download/llvmorg-13.0.0/LLVM-13.0.0-win64.exe)
4. Make sure you have `7z` in PATH.
5. Setup the environment in powershell (or set the same variables in cmd): `./__env.ps1` - I do this in VS Code built-in terminal
    - some machines might have issue that scripts are forbidden to run, try to `set-executionpolicy remotesigned` in powershell with admin
6. Compile and run: `cargo run --release`

### Building on MacOS
1. Get latest stable Rust language from: https://rustup.rs/
2. Install Xcode command line tools: `xcode-select --install`
3. Clone the repo: `git clone https://github.com/gyroflow/gyroflow.git`
4. Install dependencies: `cd gyroflow/ext && ./install-deps-mac.sh`
5. For building on Apple M1, change `ffmpeg-x64_64` -> `ffmpeg-arm64` and `x64-osx-release` -> `arm64-osx` in `__env-macos.sh`
6. Setup the environment in terminal: `./__env-macos.sh` - I do this in VS Code built-in terminal
7. For some reason `DYLD_FALLBACK_LIBRARY_PATH` may be overwritten so export it again in terminal: `export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/Toolchains/XcodeDefault.xctoolchain/usr/lib/"` (if you have build errors, try the other one from __env-macos.sh)
8. Compile and run: `cargo run --release`
9. If it fails to run, do: `./_deployment/deploy-macos.sh` once

### Building on Linux
1. Get latest stable Rust language from: https://rustup.rs/
2. Clone the repo: `git clone https://github.com/gyroflow/gyroflow.git`
3. Install dependencies: `cd gyroflow/ext && ./install-deps-linux.sh` (Debian based apt)
4. Setup the environment in terminal: `./__env-linux.sh` - I do this in VS Code built-in terminal
5. Compile and run: `cargo run --release`

### Building for Android
1. Android is not well supported yet, but the app can be built and somewhat works. So far only building on Windows was tested
2. Install Qt for Android: `aqt install-qt windows android 6.2.3 android_arm64_v8a` and `aqt install-qt windows desktop 6.2.3 win64_mingw`
3. Install `cargo-apk`: `cargo install --git https://github.com/zer0def/android-ndk-rs.git cargo-apk`
4. Add a Rust target: `rustup target add aarch64-linux-android`
5. Update `Cargo.toml` to comment out `[[[bin]]` section and uncomment `[lib]` section
6. Patch `C:\Users\you\.cargo\registry\src\github.com-1ecc6299db9ec823\opencv-0.61.3\build.rs`: Change `if cfg!(target_env = "msvc")` to `if std::env::var("CARGO_CFG_TARGET_ENV").unwrap() == "msvc"`
7. Update paths in `_deployment/build-android.ps1` and in `_deployment/android/android-deploy.json`
8. Run `.\_deployment\build-android.ps1` in Powershell

### Profiling on Windows
1. Install and run `Visual Studio Community Edition`
2. Compile and run GyroFlow with the `profile` profile: `cargo run --profile profile`
3. In Visual Studio, go to `Debug -> Performance Profiler...`
    - Under `Target`, open `Change Target` and select `Running Process...`, select the running `gyroflow.exe` process
    
## License

Distributed under the GPLv3 License. See [LICENSE](https://github.com/gyroflow/gyroflow/blob/main/LICENSE) for more information.

## Authors

* [AdrianEddy](https://github.com/AdrianEddy/) - *Author of the Rust implementation (code in this repository), author of the UI, GPU processing, rolling shutter correction and advanced rendering features*
* [Elvin Chen](https://github.com/ElvinC/) - *Author of the first version in Python, laid the groundwork to make all this possible*

### Notable contributors
* [Aphobius](https://github.com/Aphobius/) - *Author of velocity dampened smoothing algorithm*
* [Marc Roeschlin](https://github.com/marcroe/) - *Author of adaptive zoom algorithm*

## Acknowledgements

* [Gyroflow Python version (legacy code)](https://github.com/ElvinC/gyroflow)
* [telemetry-parser](https://github.com/AdrianEddy/telemetry-parser)
