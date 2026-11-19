# ESP32C6-N16 项目

## 硬件

| 项目 | 说明 |
| --- | --- |
| MCU | ESP32-C6（RISC-V） |
| Flash | 16 MB |
| RGB LED | WS2812，GPIO8 |
| 按键 | BOOT，GPIO9 |
| 调试 | 板载 ESP JTAG + `probe-rs` |

## 快速开始

```bash
rustup target add riscv32imac-unknown-none-elf
cargo install probe-rs-tools

cp .env.example .env      # 填 WIFI_SSID / WIFI_PASSWORD
cargo run --release       # 运行主应用

scripts/run-example.sh selftest   # 运行示例
```

- `cargo run --release` 默认运行主应用；示例用 `scripts/run-example.sh <name>`。
- 固件只使用 release 构建；`dev`/`test` profile 也复用 release 优化参数。
- `.env` 已忽略；密码为空表示开放网络。
- 日志通过 defmt + RTT 输出，`probe-rs run` 会直接显示。

## 文档

- [app/README.md](app/README.md)：主应用功能、HTTP、BLE NUS、测试
- [examples/README.md](examples/README.md)：示例列表与运行方式
- [crates/README.md](crates/README.md)：架构、启动顺序、版本矩阵、调试排错
