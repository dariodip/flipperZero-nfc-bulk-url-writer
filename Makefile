# NFC Bulk URL Writer — build, test and deploy helper.
#
# Common overrides:
#   make test
#   make fmt
#   make release
#   make deploy FLIPPER_PORT=/dev/cu.usbmodemflip_XXXX
#
# `storage` and `run-fap` auto-detect the Flipper when FLIPPER_PORT is empty.

SHELL := /bin/sh

FAP_NAME := nfc-bulk-url-writer
TARGET := thumbv7em-none-eabihf
HOST_TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')

DEBUG_FAP := target/$(TARGET)/debug/$(FAP_NAME).fap
RELEASE_FAP := target/$(TARGET)/release/$(FAP_NAME).fap

FLIPPER_FAP_PATH ?= /ext/apps/Examples/$(FAP_NAME).fap
FLIPPER_PORT ?=
PORT_ARG := $(if $(FLIPPER_PORT),-p $(FLIPPER_PORT),)

.PHONY: all help build test fmt fmt-check clippy check release deploy deploy-debug run clean

all: release

help:
	@echo "NFC Bulk URL Writer"
	@echo ""
	@echo "  make test         Run the host-testable core tests"
	@echo "  make fmt          Format all Rust code"
	@echo "  make fmt-check    Check formatting without changing files"
	@echo "  make clippy       Run clippy"
	@echo "  make check        fmt-check + clippy + test"
	@echo "  make build        Build the debug FAP"
	@echo "  make release      Build the release FAP"
	@echo "  make deploy       Build release and copy it to the Flipper"
	@echo "  make deploy-debug Copy the debug FAP to the Flipper"
	@echo "  make run          Build release and run it through run-fap"
	@echo "  make clean        Remove Cargo artifacts"
	@echo ""
	@echo "  FLIPPER_PORT=/dev/cu.usbmodemflip_XXXX to select the device"
	@echo "  FLIPPER_FAP_PATH=$(FLIPPER_FAP_PATH)"

build:
	cargo build

test:
	cargo test -p $(FAP_NAME)-core --target $(HOST_TARGET)

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy

check: fmt-check clippy test

release:
	cargo build --release
	@echo "Release FAP: $(RELEASE_FAP)"

deploy: release
	storage $(PORT_ARG) send $(RELEASE_FAP) $(FLIPPER_FAP_PATH)
	@echo "Deployed $(FAP_NAME) to $(FLIPPER_FAP_PATH)"

deploy-debug: build
	storage $(PORT_ARG) send $(DEBUG_FAP) $(FLIPPER_FAP_PATH)
	@echo "Deployed debug $(FAP_NAME) to $(FLIPPER_FAP_PATH)"

run: release
	run-fap $(PORT_ARG) $(RELEASE_FAP)

clean:
	cargo clean
