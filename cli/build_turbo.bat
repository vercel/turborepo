cd ..\crates\turborepo
cargo build --target x86_64-pc-windows-gnu
if %errorlevel% neq 0 exit /b %errorlevel%

copy ..\..\target\x86_64-pc-windows-gnu\debug\turbo.exe ..\..\target\debug\turbo.exe
cd ..\..\cli
