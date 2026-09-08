# 0002: API Key 存系统密钥链，配置只存 `secret://` 引用，单向写入

调用 Provider 需要凭证，README 承诺 API Key 仅存本地系统密钥链。我们决定：真实密钥只写入 OS 密钥链（macOS Keychain / Windows Credential Manager / Linux Secret Service），配置文件里 `api_key` 字段只保存两种值之一——null（未设置）或引用 URI `secret://io.github.xezzon.agent-maestro/provider/<slug>/api_key`；前端永不读回密钥本身（更新契约：`null`=不变、`{ slug, value }`=覆盖写入、`{ slug }`=清除）。

这是「单向写入（write-only）」姿态：渲染进程即使被攻破也拿不到密钥，只能看到无秘密的引用。为此拒绝了「显示/隐藏」回读密钥的常见 UX（每次渲染都把秘密送回前端，姿态明显更弱）。配置因此可移植、不含秘密；代价是密钥链条目与配置文件分离（整目录拷贝到新机器不会带走密钥链条目，留待同步功能处理）。

修订（2026-09-08）：第一期（#24）暂缓采纳本决策——`api_key` 以明文字符串直接存入配置文件，以收敛首期范围、避免密钥链在 Linux Secret Service 上的跨平台不确定性。本 ADR 的「密钥链 + `secret://` 引用」方案保留为目标状态，将来落地时需做一次数据迁移（明文值写入密钥链、替换为引用）。在此之前，配置文件以明文承载密钥是已知的、有意的取舍。
