# Secret 主副本存系统密钥链，配置以 secret_ref 引用

Maestro 的认证凭据（API Key）主副本只存系统密钥链（macOS Keychain / Windows Credential Manager / Linux Secret Service），`~/.maestro` 配置文件中不落明文，`api_key` 字段只存 `secret://<name>` 引用。投影到各工具时，凭据按工具自身的存储方式落地（对 Pi 即明文写入 `~/.pi/agent/models.json`），密钥链只保护 Maestro 的主副本，投影落地后的工具侧文件不再受 Maestro 保护。

选它而非配置内联明文，是因为 README 已对外承诺"密钥仅存本地、不落配置文件"，且密钥明文躺在磁盘上是不可逆的安全债。代价是将来做跨设备同步时，密钥不能随配置文件走，需要单独设计凭据同步（第一期不做）。

## Considered Options

- **配置内联明文**：放弃。违背安全承诺，且与文档自相矛盾。
- **混合（密钥链为主 + env/文件覆盖）**：放弃。第一期的复杂度与收益不匹配。
- **投影时用 Pi 的 `!command` 语法取 key（key 在工具侧也不落盘）**：曾考虑，最终为第一期简单起见选了投影内联明文；Pi 原生支持 `!command`，未来可平滑切换，因此不算不可逆。
