@echo off
setlocal
cargo fmt --all -- --check || exit /b 1
cargo clippy --workspace --all-targets -- -D warnings || exit /b 1
cargo test --workspace || exit /b 1
pushd admin-web
call npm.cmd ci || exit /b 1
call npm.cmd run build || exit /b 1
popd
cargo build --release --workspace || exit /b 1
echo Rust workspace and admin web build passed.
