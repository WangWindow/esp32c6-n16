# examples — 独立示例

这里的每个子目录都是一个独立的 binary crate，有自己的 `#[esp_hal::main]`，
不依赖 `app`，也不会被链接进主应用。它们通过 `crates/` 复用框架能力。

| 示例 | 内容 | 需要凭据 |
|---|---|---|
| `selftest` | 芯片/MAC、heap、LED、按键、温度、Wi-Fi 扫描、BLE 控制器 | 否 |
| `wifi` | Wi-Fi STA + DHCP + HTTP JSON 状态 | 是（无则退回扫描） |
| `ble` | BLE NUS 外设 | 否 |

## 运行

```bash
# 推荐：自动加载 .env
scripts/run-example.sh selftest
scripts/run-example.sh wifi
scripts/run-example.sh ble

# 或者直接指定 package
cargo run -p esp32c6-n16-example-selftest --release
cargo run -p esp32c6-n16-example-wifi --release
cargo run -p esp32c6-n16-example-ble --release
```

`wifi` 示例的 HTTP 端点：

```bash
curl http://<board-ip>/
```

## 新增示例

1. 复制 `examples/wifi/`，修改 package/bin 名称和 `main`；
2. 在根 `Cargo.toml` 的 `members` 中加入新目录；
3. 在 `scripts/check.sh` 的 `PACKAGES` 中加入 package 名；
4. 示例只依赖 `crates/`，不要依赖 `app`。
