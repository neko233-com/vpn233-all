@echo off
setlocal
cargo build --release -p vpn233-provider-server -p vpn233-subscribe-server || exit /b 1
if not exist generated mkdir generated
copy /Y target\release\vpn233-provider-server.exe generated\vpn233-provider-server.exe
copy /Y target\release\vpn233-subscribe-server.exe generated\vpn233-subscribe-server.exe
echo Built the Rust provider and subscribe services. Cross-platform release bundles are produced by GitHub Actions.
