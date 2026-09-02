# 0002: API Key 存系统密钥链，配置只存 `secret://` 引用，单向写入

调用 Provider 需要凭证，README 承诺 API Key 仅存本地系统密钥链。我们决定：真实密钥只写入 OS 密钥链（macOS Keychain / Windows Credential Manager / Linux Secret Service），配置文件里 `api_key` 字段只保存两种值之一——空串（未设置）或引用 URI `secret://io.github.xezzon.agent-maestro/provider/<slug>/api_key`；前端永不读回密钥本身（更新契约：`null`=不变、字符串=覆盖写入、空串=清除）。

这是「单向写入（write-only）」姿态：渲染进程即使被攻破也拿不到密钥，只能看到无秘密的引用。为此拒绝了「显示/隐藏」回读密钥的常见 UX（每次渲染都把秘密送回前端，姿态明显更弱）。配置因此可移植、不含秘密；代价是密钥链条目与配置文件分离（整目录拷贝到新机器不会带走密钥链条目，留待同步功能处理）。
