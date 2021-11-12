# gyroflow
Gyroflow Rust port, based on the original work of ElvinC `https://github.com/ElvinC/gyroflow/`


# Dev environment
Visual Studio Code + `rust-analyzer` extension
Optionally `CodeLLDB` extension for debugging

# Building
1. Get latest stable Rust language from: https://rustup.rs/
2. Install Qt 6.2 or higher: https://www.qt.io/download-qt-installer
3. Clone the repo: `git clone https://github.com/AdrianEddy/gyroflow.git`
4. Download `FFmpeg`, `OpenCV` and `llvm` and put them in `ext` directory according to paths in `__env.ps1`: 
4. a) https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full-shared.7z
4. b) https://sourceforge.net/projects/opencvlibrary/files/4.5.4/opencv-4.5.4-vc14_vc15.exe/download
4. c) https://github.com/llvm/llvm-project/releases/download/llvmorg-13.0.0/LLVM-13.0.0-win64.exe
5. Update Qt path in `__env.ps1`
6. Setup the environment in powershell (or set the same variables in cmd): ./__env.ps1 - I do this in VS Code built-in terminal
7. Compile and run: `cargo run --release`

