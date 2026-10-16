@echo off
cd /d "D:\Downloads\Marguerite\marguerite\gui\src-tauri"
set "CARGO_TARGET_DIR=D:\Downloads\Marguerite\marguerite\marguerite_build_target"
set "RUST_BACKTRACE=1"
cargo tauri build --verbose
