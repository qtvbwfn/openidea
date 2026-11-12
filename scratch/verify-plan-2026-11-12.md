# 完整版真机验证计划（2026-11-12，release d9536c96，0.1.40）

产物：openidea-verify/target/release/bundle/macos/OpenIDEA.app（+DMG）

## A. 冒烟
1. 启动 .app，扫描 MCP 桥端口 9223-9322（node WebSocket，{id,command,args} 协议）
2. execute_js 确认 window 全局、identifier（release 身份，非 .dev.w*）、数据目录 ~/.openidea

## B. fork 功能回归（86 提交 → 11 域，重点 4 个）
- [ ] **配置目录 v3.5**：首启不弹迁移（已迁过）；设置页 app_home 行显示 ~/.openidea + isDefault
- [ ] **MyBatis intentions**：打开 fixtures/mybatis-app → UserMapper.java → ⌥↵ 三意图**无条件**出现（881ada6d/a1e6758f 语义：点击时判断 jump/create）；空 mapper 接口 Create Mapper XML… 能建文件；建完 XML 菜单不消失（合并索引而非重置缓存）；gutter 双向箭头
- [ ] **Services opt-in（d9536c96）**：Services 窗口**无默认 Docker 连接**；手动 Add Service → 出现；Remove → 立即消失；重启后仍只有手动的
- [ ] **Services 分组/批量**（0b8acc6c）：Add Service 分组菜单；组级 Run All/Restart All/Stop All
- [ ] **语义索引 M0-M3**：打开 vue-app → 无 LS 时 ⌘B 走 index goto（fallback note "based on project index"）；Vue outline script 声明
- [ ] **TODO 就地编辑**：TODO 窗口点击预览可编辑（opt-in 开关）
- [ ] **Run tab 图标**（05b58df7）：会话结束后 tab 显示配置自身图标
- [ ] **README 双语**：仓库内 README.zh-CN.md 在

## C. 插件系统（Agent B 4 步路径）
- [ ] **启动加载**：Settings›Plugins›Installed 状态 Running/Enabled/Inactive，Statistics activations
- [ ] **安装（release 版无内置）**：齿轮 "Install Plugin from Disk…" → openidea-verify/plugins/official/regex-tester → 权限页 "Install Regex Tester?" → Installed +1
- [ ] **菜单触发**：编辑器选中文本右键 "Regex Tester: Test Selection" → 底部 Regex Tester view；Tools 菜单 "License Header: Find Files Without a Header"
- [ ] **开关**：插件 Disable → 菜单/action 消失 → Enable 恢复
- 可选：marketplace Available 页能加载 OFFICIAL_INDEX

## D. 上游新功能抽验（代理 A 路径精选）
- [ ] **agents.mcpServers**：Find Action → Settings›Agents›MCP 页存在，Import .mcp.json 可用
- [ ] **JUnit run config**：run.editConfigs → + → JUnit 类型存在（java-app）
- [ ] **终端 Ctrl/Cmd+滚轮**：Settings›Editor›General 开 wheelZoom → 终端字号变化
- [ ] **插件 API 1.5**：Settings›Plugins 里看两插件 depends 的 Blocked 展示（若装了依赖插件对）

## 判定
- 每项记录：通过/失败 + 截图/日志证据
- 失败项按 issue-fix 流程走；全部结果写入记忆
