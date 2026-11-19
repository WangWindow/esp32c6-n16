# app — 主应用

`app` 是独立的主应用 binary。
它通过共享 crates 组合出可实际使用的 Wi-Fi + BLE 设备程序。

## 功能

- Wi-Fi station：连接构建期配置的 AP、DHCP、断线重连。
- HTTP 状态端点：`GET /` 返回 JSON。
- 公开数据：通过 Open-Meteo HTTP API 获取天气和当地时间，随状态一起返回。
- BLE NUS：Nordic UART Service，手机写入 RX，设备通过 TX 通知响应。
- Wi-Fi 与 BLE 通过 `esp-radio` 的 `coex` feature 同时运行。
- 状态 LED：连接中青色、已连接绿色、断开/失败红色。

## 运行

```bash
# 主应用是 workspace 默认 member
cargo run --release
# 或加载 .env 后运行
scripts/run.sh
```

Wi-Fi 凭据：

```dotenv
# .env（已忽略）
WIFI_SSID=MyAP
WIFI_PASSWORD=secret
```

密码为空表示开放网络。没有凭据时 Wi-Fi 退回扫描模式，BLE 仍正常工作。
`cargo run` 不会自动加载 `.env`，需要脚本或手动导出环境变量。

## HTTP 状态

DHCP 成功后：

```bash
curl http://<board-ip>/
# {"temperature_c":36,"rssi":-48,"uptime_s":42,
#  "weather_temperature_c":8,"weather_code":0,"local_time":"18:45"}
```

实现位于 `crates/wifi` 的 `serve_status`，只使用 `embassy-net` TCP socket 和
`heapless::String`，没有引入 HTTP 框架。

## 天气与时间

`crates/wifi::fetch_weather` 通过 `http://api.open-meteo.com` 获取：

- 当前温度、WMO 天气代码；
- 当地时间（由 `timezone=auto` 返回）；
- 采用最小的 HTTP/1.1 + 手动 chunked 解码，不依赖 TLS/HTTP 框架。

默认坐标是合肥（`31.82, 117.23`），可在构建时覆盖：

```bash
WEATHER_LAT=39.90 WEATHER_LON=116.40 cargo run --release
```

启动后等待 DHCP，随后立即拉取一次；成功后每 15 分钟刷新，失败则 30 秒后重试。

## BLE NUS

广播名 `ESP32-C6-N16`。

| 角色 | UUID |
|---|---|
| Service | `6e400001-b5a3-f393-e0a9-e50e24dcca9e` |
| RX（写） | `6e400002-b5a3-f393-e0a9-e50e24dcca9e` |
| TX（通知） | `6e400003-b5a3-f393-e0a9-e50e24dcca9e` |

用 nRF Connect 等客户端先打开 TX 通知，再写 RX：

- `ping` → `pong\n`
- `status` → `esp32c6-n16\n`
- 其他数据 → 原样回显（最多 20 字节）

## 启动流程

`main` 只做两件事：调用共享 runtime，然后运行业务 future。

```rust
#[esp_hal::main]
async fn main(_spawner: Spawner) -> ! {
    let board = esp32c6_n16_runtime::init("app");
    // Wi-Fi + BLE NUS + HTTP
}
```

`crates/runtime` 负责 HAL、日志、heap、RTOS、Board 的固定顺序；细节见
[`crates/README.md`](../crates/README.md)。

## 测试

```bash
cargo test -p esp32c6-n16-app --release --test hardware
```

板上测试覆盖芯片/MAC、heap、温度、按键、RGB LED。测试使用独立的
`app/embedded-test.x` 链接脚本，不会进入固件。

## 调试

```bash
DEFMT_LOG=debug scripts/run.sh
DEFMT_LOG='esp32c6_n16_wifi=trace,esp_radio=debug' scripts/run.sh
```

Zed 的 task/debug 只针对本 app：

- Build：`cargo build --release`
- Run：`cargo run --release`
- ELF：`target/riscv32imac-unknown-none-elf/release/esp32c6-n16-app`

常见问题（Wi-Fi 扫描为空、BLE 地址、NUS 通知、HTTP 无响应、heap）见
[`crates/README.md`](../crates/README.md) 的“调试与排错”。
