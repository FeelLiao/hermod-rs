# Hermod CLI：设备配对与按 ID 接收

## 首次配对

双方先通过 `hermod trust SERVER --fingerprint HEX` 选择同一个可信信令服务器。
设备功能需要新版 Rust server；服务器仍不接收文件内容。

```text
# 发送端
hermod tx ./hello.txt --pair

# 接收端：使用发送端显示的完整传输码
hermod rx CODE --pair
```

两端必须显式同意。普通码传输不创建联系人，也不更新已有配对密钥。
配对身份和持久化确认在文件数据面建立前完成；文件传输失败不等于配对失败。
已有相同配对重复执行不轮换密钥、不覆盖本机备注。

## 设备 ID 与备注

```text
hermod device id
hermod device list
hermod device rename 0123456789abcdef0123456789abcdef "我的笔记本"
hermod device remove 0123456789abcdef0123456789abcdef
```

ID 是规范公钥指纹，固定 32 个小写十六进制字符。备注只在本机保存；改备注不会
改变 ID、公钥或对端名称。允许重名，但按备注查找不唯一时必须使用完整 ID。
列表只显示 ID、备注和配对状态，不显示私钥、配对密钥或完整联系人 JSON。

## 后续接收

```text
# 发送端仍需主动选择本次发送的内容
hermod tx ./archive.zip

# 接收端二选一
hermod rx --from 0123456789abcdef0123456789abcdef
hermod rx --from "我的笔记本"
```

`rx CODE` 与 `rx --from DEVICE` 互斥，`--from` 不能与 `--pair` 混用。
无后台常驻、远程文件浏览或任意文件拉取。一个本机身份只允许一个发送任务，
第二个发送任务立即报忙。`-4/-6` 同时约束码连接、设备控制连接和 QUIC。

发送时保留传输码连接，并另建 `/device` WSS 控制连接。ID 候选需通过已保存配对
密钥的 CPace 和双方长期签名认证，才可成为唯一接收方；未认证呼叫不会作废码入口。
不支持设备控制的服务器仍能用于普通传输码。设备控制连接失败或断开不会将 ID
认证降级为传输码；需查看控制入口诊断时使用 `--verbose warning` 或更详细级别。

## 本机存储与安全

文件位于现有配置目录下独立的 `device-state/`：

```text
device-state/
├── device.json       # 长期身份，包含私钥
├── contacts.json     # 配对秘密、备注、pending/active 状态
├── identity.lock
├── contacts.lock
└── send.lock
```

Windows 根目录通常为 `%APPDATA%\Hermod`，Linux 为 `$XDG_CONFIG_HOME/hermod`
或 `~/.config/hermod`。不会修改原有配置根目录的权限。

- Linux：独立目录 0700、文件 0600，并验证所有者。其他 Unix 平台未验证，不作支持承诺。
- Windows：私密目录和文件限制为当前用户与 SYSTEM，显式验证 Windows 文件权限。
  当前实现通过系统 PowerShell 管理和检查权限，有启动开销；没有 DPAPI 加密。
- 身份缺失时才创建；损坏、版本不支持、权限不安全时明确失败，不静默生成新身份。
- 随机临时文件、文件锁和原子替换保护写入；配置父目录须可信，不声称防御同用户恶意进程。
- 不要上传或分享这些文件；其中的密钥不是公开设备 ID。
- 配对待确认状态可在双方再次显式 `--pair` 时核对恢复；active/缺失或记录冲突时要求
  双方删除旧联系人后重新配对，不隐式重置信任。
- 同一配置目录的身份在不同服务器上保持一致，因此服务器可关联同一 ID 的活动。

传输调优依旧只从本次命令行和程序默认值取得，不写入这些持久文件。

## 协议边界

注册证明只证明调用连接持有该设备私钥，不证明该设备值得信任。联系人验证在两端进行。
控制服务不持久化设备信息，但可观察在线 ID、呼叫关系和网络地址。

QUIC payload v3 未改。当前接收端向发送端回传摘要裁决的行为保持不变；设备配对确认
不等于增加了发送端返回文件摘要确认的新载荷帧。
