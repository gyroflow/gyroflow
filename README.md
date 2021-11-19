# gyroflow
Gyroflow Rust port, based on the original work of ElvinC `https://github.com/ElvinC/gyroflow/`

# Code structure
1. Entire GUI is in the `src/ui` diretory
2. `controller.rs` is a bridge between UI and core, it takes all commands from QML and calls functions in core
3. `core` directory contains the whole gyroflow engine and doesn't depend on Qt or ffmpeg, and OpenCV is optional
4. `rendering` contains all FFmpeg related code for rendering final video and processing (for synchronization)
5. `core/gpu` contains GPU implementations of the undistortion
6. `mod.rs` in each directory acts as a main entry of the module (directory name is the module name and `mod.rs` is the kind of an entry point)
7. `main.rs` contains a TODO list of things that still need to be done. Also there's a ton of TODO commends throughout the code


# Dev environment
Visual Studio Code + `rust-analyzer` extension.
Optionally `CodeLLDB` extension for debugging

For working with QML I recommend to use Qt Creator and load all QML files there, as it has auto-complete and syntax highlighting.
The project also supports UI live reload, it's a super quick way of working with the UI. Just change `live_reload = true` in `main.rs` and it should work right away. Now every tim you change any QML file, the app should reload it immediately.

# Building on Windows
1. Get latest stable Rust language from: https://rustup.rs/
2. Install Qt 6.2 or higher: https://www.qt.io/download-qt-installer
3. Clone the repo: `git clone https://github.com/AdrianEddy/gyroflow.git`
4. Download `FFmpeg`, `OpenCV` and `llvm` and put them in `ext` directory according to paths in `__env.ps1`: 
5. - https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full-shared.7z
6. - https://sourceforge.net/projects/opencvlibrary/files/4.5.4/opencv-4.5.4-vc14_vc15.exe/download
7. - https://github.com/llvm/llvm-project/releases/download/llvmorg-13.0.0/LLVM-13.0.0-win64.exe
8. Update Qt path in `__env.ps1`
9. Setup the environment in powershell (or set the same variables in cmd): `./__env.ps1` - I do this in VS Code built-in terminal
10. Compile and run: `cargo run --release`

# Building on MacOS
1. Get latest stable Rust language from: https://rustup.rs/
2. Install Xcode command line tools: `xcode-select --install`
3. Install Qt, OpenCV and 7z: `brew install qt opencv p7zip`
4. Download ffmepg from avbuild: https://sourceforge.net/projects/avbuild/files/macOS/ffmpeg-4.4-macOS-lite.tar.xz/download and extract to `ext` directory
5. Clone the repo: `git clone https://github.com/AdrianEddy/gyroflow.git`
6. Update paths in `__env-macos.sh`
7. Setup the environment in terminal: `./__env-macos.sh` - I do this in VS Code built-in terminal
8. Compile and run: `cargo run --release`

