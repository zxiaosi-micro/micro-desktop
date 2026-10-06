# micro-desktop 常用命令（S0-02；src-tauri 脚手架于 S9 落地）
SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

.DEFAULT_GOAL := help
.PHONY: check clippy fmt build clean help

CARGO_DIR := src-tauri

_guard:
	@test -d $(CARGO_DIR) || { echo "src-tauri 未落地（S9）：脚手架随 S9 任务创建"; exit 1; }

## check: cargo check
check: _guard
	cargo check

## clippy: cargo clippy（警告即失败）
clippy: _guard
	cargo clippy -- -D warnings

## fmt: cargo fmt
fmt: _guard
	cargo fmt --all

## build: cargo build --release
build: _guard
	cargo build --release

## clean: cargo clean
clean: _guard
	cargo clean

## help: 目标清单
help:
	@echo "check / clippy / fmt / build / clean（src-tauri 于 S9 落地）"
