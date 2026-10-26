# Hermod 微软商店素材上传说明

适用应用：Hermod · FeelLiao · Store ID `9PMPWN3HMR3M`。

在仓库根目录运行 `npm run store:prepare --prefix hermod-app`，生成目录为
`store-output/hermod-<版本>/`，同目录旁还会生成可下载的 ZIP 素材包。
素材包仅用于手动填写商店页面，不会加入 GitHub Release 的程序产物。

首次生成前需要 Node.js 22、Python 3、Chromium 和中文字体。在 Linux 上准备：

```bash
npm ci --prefix hermod-app
npm exec --prefix hermod-app -- playwright install chromium
sudo apt-get install fonts-noto-cjk
npm run store:prepare --prefix hermod-app
```

## 文字怎么填

在 Partner Center 的商店列表中分别选择 **简体中文** 和 **English (United States)**：

| 商店字段 | 中文文件 | 英文文件 |
| --- | --- | --- |
| 简短描述 | `copy/zh-CN/short-description.txt` | `copy/en-US/short-description.txt` |
| 描述 | `copy/zh-CN/description.txt` | `copy/en-US/description.txt` |
| 产品功能 | `copy/zh-CN/features.txt` | `copy/en-US/features.txt` |
| 其他最低系统要求 | `copy/zh-CN/system-requirements.txt` | `copy/en-US/system-requirements.txt` |
| 更新说明（仅更新提交时使用） | `copy/zh-CN/whats-new-for-updates.txt` | `copy/en-US/whats-new-for-updates.txt` |

文本均为可复制的纯文本；功能和系统要求按一行一项分别填写。
首次提交时“此版本中的新增功能”留空，更新提交时才使用更新说明。
展示名称使用 **Hermod**，并核对它与 Partner Center 中保留的产品名称一致；
包标识 `FeelLiao.HermodTransfer` 用于程序包身份。

## 图片怎么上传

| 字段 | 文件 | 规格 |
| --- | --- | --- |
| 桌面截图（简体中文） | `screenshots/zh-CN/01-*.png` 至 `06-*.png` | 每张 1920×1080 PNG |
| 桌面截图（英文） | `screenshots/en-US/01-*.png` 至 `06-*.png` | 每张 1920×1080 PNG |
| 1:1 应用磁贴图标 | `logos/app-tile-300.png` | 300×300 PNG，沿用应用现有标志 |
| 16:9 Super hero art（可选） | `promotional/hero-1920x1080.png` | 1920×1080 PNG，无文字 |

每种语言上传对应的 6 张截图，按编号排序。图片说明位于对应语言的
`copy/<语言>/screenshot-captions.txt`，逐条复制到截图标题/说明字段。
图标和宣传图可在两个语言列表中复用。

`sources/app-icon.svg`、`sources/hero.svg` 和 `logos/app-icon-1024.png` 是矢量源文件或高分辨率备用图标，
无需上传到不匹配尺寸的字段。`PREVIEW.html` 仅供本地浏览素材，不是商店图片。
本应用面向 PC 桌面；不要把这些素材填入 Xbox、全息设备或移动端截图栏。

## 截图内容说明

截图由应用当前共享的 Fluent 桌面界面直接渲染，截图时不修改页面样式、不叠加宣传标语，
也不伪造 Windows 窗口边框。截取在 Linux Chromium 中进行，未运行或构建 Windows 客户端。
演示文件名、传输码、收件箱项目与文档用 IPv6 地址由独立截图脚本提供，不来自个人文件或真实用户会话。
等待连接画面展示的是流程示例，不是传输测速结果。
如后续改动实际界面，应重新生成对应版本的图片。

图标沿用 `hermod-app/icon.svg`，宣传图为同色系的文件直传矢量插图。
商店文字已说明双方需要可达 IPv6、实际速度受网络影响及当前没有断点续传。

## 素材和程序包是两回事

此 ZIP 是供你整理上传内容的素材包，不是 MSIX，不应上传到“程序包 / Packages”栏。
程序包栏上传 GitHub Actions 产出的 `hermod-windows-x64.msix` 和 `hermod-windows-arm64.msix`。
商店文案和截图应分别添加到对应语言的商店列表中。

## 官方参考（2026-10-26 核对）

- [截图、图标与宣传图](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/screenshots-and-images)：桌面截图至少 1366×768，PNG 小于 50 MB；建议提供多张，截图不加额外营销文字；图标槽位为 300×300，宣传图不含文字。
- [商店列表文字字段](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/add-and-edit-store-listing-info)：描述、功能、简短描述和更新说明分别填写。

商店是否展示宣传图及最终审核结果由 Microsoft 决定；准备素材不等于提交或发布。
