@echo off
setlocal
cargo build --release -p vpn233-client || exit /b 1
if not exist generated mkdir generated
copy /Y target\release\vpn233-client.exe generated\vpn233-client.exe
echo Built the Rust client. Cross-platform release bundles are produced by GitHub Actions.
