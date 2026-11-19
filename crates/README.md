# crates — 共享框架

`app` 和 `examples/` 都只通过这里的 crate 复用能力。芯片固定为 ESP32-C6，
因此没有做跨芯片泛型抽象。

| crate | 职责 |
|---|---|
| `runtime` | HAL/日志/heap/RTOS/Board 的统一启动序列 |
| `bsp` | `Board`、WS2812 LED（GPIO8）、BOOT 键（GPIO9）、温度、板信息 |
| `wifi` | 扫描、STA、DHCP、HTTP 状态、coex |
| `ble` | BLE 控制器、NUS GATT 外设 |
| `log` | defmt 时间戳、启动横幅 |
| `testkit` | 轻量自检/断言套件 |

## 启动顺序（runtime）

```text
esp_hal::init
  └── logging init
      └── heap_allocator! x2     # reclaimed DRAM2 + 64 KiB 内部 RAM
          └── esp_rtos::start(timg0.timer0, FROM_CPU_INTR0)
              └── Board::new      # RMT/GPIO8/GPIO9/TSENS/WIFI/BT
                  └── boot banner
                      └── return Board
```

- 射频必须在 `esp_rtos::start` 之后初始化。
- heap 必须在创建 Wi-Fi/BLE 控制器之前准备好。
- `esp_bootloader_esp_idf::esp_app_desc!()`、`defmt-rtt`、`esp-backtrace` 由每个
  binary 自己链接；runtime 不包含入口点。

## 外设所有权

| 外设 | 所有者 |
|---|---|
| `TIMG0` / `FROM_CPU_INTR0` | `esp-rtos` |
| `RMT` / `GPIO8` | `StatusLed` |
| `GPIO9` | `Button` |
| `TSENS` | `Temperature` |
| `WIFI` | `WifiService` |
| `BT` | `ble::run_peripheral` |

`WIFI` 与 `BT` 可同时持有；`esp-radio` 的 `coex` feature 负责射频仲裁。

## Feature 裁剪

- `crates/wifi`：`wifi` feature 基础；`net` 才引入 `embassy-net`/`StaticCell`；
  `coex` 才启用 Wi-Fi + BLE 共存。
- `crates/ble`：只做外设/NUS，不链接 central/scanner。
- 主 app 使用 `net + coex + ble`；`examples/wifi` 只开 `net`；
  `examples/ble` 只链接 BLE；`examples/selftest` 使用双射频自检。

## Wi-Fi

- station 配置通过 `ControllerConfig::with_initial_config` 传入。
- 扫描使用 100–150 ms/信道的主动扫描。官方示例默认 10–20 ms，在拥挤环境实测
  会只返回 0–1 个 AP，因此做了有依据的偏离。
- `connection_loop`：连接、等待断开、5 秒退避重连，并通过回调上报 RSSI/状态。
- `start_network` 返回 `embassy-net` 的 `Stack` + `Runner`；`monitor` 打印 DHCP
  地址变化。
- `serve_status` 在端口 80 提供 JSON 状态，只用 TCP socket + `heapless::String`。

## BLE

- `esp-radio` `BleConnector` + `trouble-host` 的 `ExternalController`、
  `HostResources`、GATT derive 宏。
- static random address 的最高两 bit 必须设置在线上地址的最高字节
  （`mac[5] |= 0xC0`），否则控制器返回 `Invalid HCI Command Parameters`。
- peripheral 用 `join(runner, advertise)`，HCI 命令必须有 host runner 并发处理。
- NUS：RX 写、TX 通知；`ping` → `pong`，`status` → 设备名，其他数据回显。

## 日志

- 统一 `defmt` + RTT；不混用 `log`/`esp-println`。
- `DEFMT_LOG` 是编译期过滤，修改后需重新构建。
- 启动横幅包含应用名、git hash、profile、芯片版本、MAC、复位原因、heap。

## 构建约束

- `opt-level = 3`、`lto = "fat"`、`codegen-units = 1`；射频代码需要优化。
- 仅 release 工作流；`dev`/`test` 复制 release 参数并关闭 DWARF/overflow checks。
- 固件/库 target：`test = false`、`bench = false`、`harness = false`。
- `Cargo.lock` 提交；`rust-toolchain.toml` 固定工具链。
- 各 crate `#![forbid(unsafe_code)]`、`#![deny(missing_docs)]`；
  `clippy::unwrap_used`/`expect_used` 在 CI 中即错误。

## 版本矩阵

| crate | 版本 |
|---|---|
| esp-hal | `=1.2.2` |
| esp-radio | `=1.0.0-beta.1` |
| esp-rtos | `=0.4.0` |
| esp-alloc | `=0.11.0` |
| trouble-host | `=0.7.0` |
| embassy-executor / net / time | `0.10` / `0.9.1` / `0.5.1` |

`esp-radio 1.0.0-beta.1` 是当前面向 esp-hal 1.2 的已发布版本；`0.18` 依赖
esp-hal 1.1。升级时请成组升级并重跑全部验收。

## 调试与排错

### Wi-Fi 扫描返回 0 个 AP

默认 10–20 ms 太短；本仓库使用 100–150 ms。确认 AP 是 2.4 GHz（C6 不支持
5 GHz）、天线正常，并用 `DEFMT_LOG=esp_radio=debug` 看驱动日志。

### BLE 广播失败 / `Invalid HCI Command Parameters`

静态随机地址最高两 bit 必须为 `11`，且设置在线上地址的最高字节
（`mac[5] |= 0xC0`）。

### BLE NUS 没有通知

TX 必须被中心订阅（CCCD）。先打开 notify 再写 RX；用
`DEFMT_LOG=trouble_host=debug` 看 GATT/CCCD 事件。

### HTTP 没有响应

日志中是否有 `net: ipv4 ...`；电脑与板子是否同网段；端口固定 80；
用 `curl -v http://<ip>/` 排查。

### heap / 射频初始化失败

确认 heap 和 `esp_rtos::start` 都在射频初始化之前；主 app 的阈值在
`crates/runtime/src/lib.rs`（64 KiB + reclaimed 64 KiB），可增大
`INTERNAL_HEAP_BYTES`。

### 日志看不到

本项目只用 RTT/defmt，不经过 UART/USB Serial-JTAG。请用 `probe-rs run`，
不要用串口监视器。

## 测试

- `scripts/check.sh`：fmt、app/示例 clippy、release 构建、测试编译。
- `cargo test -p esp32c6-n16-app --release --test hardware`：
  embedded-test + probe-rs，每个测试前复位。
- 端到端：`app` 验证 HTTP + NUS + coex；`examples/selftest` 验证射频初始化；
  `examples/wifi`/`examples/ble` 分别验证单射频路径。
