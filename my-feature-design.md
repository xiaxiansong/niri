# niri 特性设计（最终版）：移除后收紧视口 & 全局暂存区

- 定稿日期：2026-10-05
- 源码基线：`ed22699d`（main，2026-10-01）。文中行号均为“约”，动手前先 `git log` 确认偏移，以函数名为准。
- 输入：原设计文档（Claude Docs「niri 原生功能设计：关窗不留白 & 全局暂存区」）、Claude 复查意见、`NIRI-DESIGN-REVIEW.md`（ChatGPT 复查，下称 R1–R16）。本文是三者交叉核对源码后的最终版本，**实现以本文为准，原设计文档作废**。
- 读者：负责实现的会话。本文力求自洽：每条行为都给出规则、落点和验收方式。遇到本文未覆盖的情况，先补文档再写代码。

---

## 0. 交叉复查结论

### 0.1 两份复查的共同结论（均已源码确认，本文已采纳）

| 问题 | 结论 | 本文处理位置 |
|---|---|---|
| IPC 不按结构体比较差异 | `src/ipc/server.rs` `ipc_refresh_windows()` 约 721 行只逐项比较 `workspace_id / is_floating / title / app_id` | §4.9 |
| 首次呼出不是浮动尺寸 | `floating.rs` 约 430 行：无 `floating_window_size` 时用 `expected_size()`，即平铺尺寸 | §4.5.1 |
| 帧回调/可见性缓存 | 主扫描输出只对 `windows_for_output()` 更新（`niri.rs` 约 4956）；隐藏窗口缓存会残留，导致 idle inhibit 不释放、按刷新率收帧回调；另有 1 秒兜底定时器（`niri.rs` 约 5268）经 `with_windows_mut` 发低频回调 | §4.8 |
| 浮动窗口移除也触发夹紧 | `Layout::remove_window()` 同时处理浮动窗口 | §3.4（`!removed.is_floating`） |
| 子窗口/对话框 | 隐藏父窗口后其新对话框走 `AddWindowTarget::NextTo(parent)`，`Layout::add_window()` 约 1035 行 `unwrap()` → **panic** | §4.6 |
| 跟随破坏动态工作区不变量 | `monitor.rs` 约 2144 行要求末尾工作区为空 | §4.7 |
| xdg-activation 语义 | 过期令牌直接忽略，urgent 来自 `UrgentOnlyMarker` 或 `on-xdg-activate` 规则 | §4.10 |
| MRU 无需改构建逻辑 | `mru.rs` 约 585 行从 `layout.workspaces()` 构建，天然排除隐藏窗口；但已打开的 MRU 需删缩略图 | §4.8 |
| 测试用例 9 的区间断言 | `L > R` 时 `[L,R]` 为空 | §3.6 |
| 拖动测试 | `InteractiveMoveBegin` 只进入 Starting，需 `InteractiveMoveUpdate` 越过阈值 | §3.6 |

### 0.2 仅一方提出、经源码确认后采纳

| 来源 | 问题 | 处理 |
|---|---|---|
| ChatGPT R5 | `stop_move_animations()` 只清 x/y 移动动画；`update_window()` 可能新建 resize 动画 | §4.5 新增 `Tile::stop_all_animations()`，隐藏期间每次 update 后清理 |
| ChatGPT R7 | 只查 `pending_sizing_mode()` 不够 | §3.3 同时查 `sizing_mode()` |
| ChatGPT R8 | 手势端点不是全局合法区间；首列 left snap 不保证最左 | §3.2 改名“首末列吸附端点”，`clamp` 前保证 `left < right` |
| ChatGPT R11 | 激活与切换需拆接口，通知/任务栏不能意外隐藏窗口 | §4.4 `activate_window` 只呈现不隐藏 |
| ChatGPT R12 | 隐藏中 `remove-window-from-scratchpad` 未定义；退出暂存区要在转换真正发生后 | §4.3、§4.5.5 |
| ChatGPT R13 | 隐藏窗口仍需 refresh/configure | §4.5.6 |
| ChatGPT R15 | 副作用应统一入口、仅成功时执行 | §4.8 |
| ChatGPT §六 | 无效正则需报错；`Layout<W>` 泛型拿不到 app-id；按键重复 | §5 |
| ChatGPT §三.2 | 交互式 resize 期间也不应自动移动视图 | §3.3 |
| Claude | 拖走窗口后源工作区留白 | §3.5 F1-5（注意：不能放在 `dnd_scroll_gesture_end`，见 §3.5） |
| Claude | 合并进列（consume）同样留白 | §3.5 F1-6 |
| Claude | 隐藏前 `set_window_floating` 会让收起动画从错误位置开始 | §4.5.1 改为直接从平铺层移除，不再先切浮动 |
| 本次核对新增 | 所有带 `interactive_move` 前置检查的 `Layout` 方法，若用 id 查找后 `unwrap()`，传入隐藏窗口 id 会 panic（如 `center_window`、`toggle_window_width`、`move_floating_window`、`toggle_fullscreen` 等） | §4.5.7 统一加隐藏前置检查 |
| 本次核对新增 | 预提交钩子 `xdg_shell.rs` 约 1498 行必须能找到隐藏窗口，否则每次提交都报错 | §4.5.7 A 类 |
| 本次核对新增 | 拖动开始到结束期间，`dnd_scroll_gesture_end()` 先于窗口重新放回执行（`mod.rs` 约 4167 行） | §3.5 F1-5 的落点选择 |

### 0.3 未采纳 / 调整

| 来源 | 建议 | 结论与理由 |
|---|---|---|
| Claude | 显示中的暂存窗口改放 Monitor 级“粘性浮动层” | **不采纳**。niri 的所有聚焦类动作都经 `active_workspace_mut()` 分发，Monitor 级层要改动的路径远多于“所有权转移”。保持“显示中即活动工作区的普通浮动 Tile + 标记”。 |
| ChatGPT R1 | 窗口组：隐藏时带走子窗口，隐藏期间的新子窗口继承隐藏 | **调整**为更简单的规则：有可见子窗口时拒绝隐藏；隐藏窗口出现新子窗口时先呈现父窗口（§4.6）。理由：不需要持久的组记录；也避免“隐藏中关闭程序弹出的保存确认框被藏起来、程序看似卡死”。 |
| ChatGPT R6 | 引入“移除原因”枚举 | 部分采纳：调用点显式触发 + `RemovedTile::is_floating` 过滤 + `remove_window_impl(fill_view)`。目前只有一个需要排除的入口（拖动开始），不值得引入枚举。 |
| ChatGPT R7 | 为异步尺寸变化定义一次延迟校正 | **v1 不做**。移除不会改变其余列的 `ColumnWidth`。真正的异步场景是“退出全屏尚未提交”，该场景已通过同时检查两种 sizing mode 跳过。列入 §8 后续项。 |
| 原设计（用户已定 #4） | 非活动工作区夹紧不播动画 | **修订为总是动画**。上游 `remove_column_by_idx()` 在非活动工作区也照常动画；Overview 和工作区切换动画中，“非活动”工作区可能可见，直接写 Static 会产生可见跳变。动画基于时钟，不可见时开销可忽略。**需用户知悉**。 |
| 原设计（用户已定 #3） | 专门的收起/展开动画 | **推迟到 M5**。v1 隐藏即时完成；呈现复用现有 `window-open` 动画。理由见 R16：可打断、跨屏和 custom-shader 的连续性需要单独设计。**需用户知悉**。 |
| 原设计 | “通用键同一时刻最多显示一个” | **删除这一承诺**（R11 指出不成立），改为 §4.3 的精确规则。 |
| 原设计 | 钉住渲染 | 推迟到 M5，并按 R10 要求统一渲染与输入几何（§8）。 |

---

## 1. 范围

### 1.1 本轮（M1–M4）交付

1. **功能一：移除后收紧视口**（配置项 `layout { fill-view-after-removal; }`，默认关）。
2. **功能二：全局暂存区核心**：隐藏 / 呈现 / 切换 / 解除；按 id 操作；IPC 动作与字段；副作用统一处理；同显示器跟随。

### 1.2 明确不在本轮

- 钉住渲染（工作区切换时窗口视觉上不动）。v1 中，跟随的窗口会随新工作区一起滑入。
- 专门的收起/展开动画（v1：隐藏即时完成；呈现用 window-open 动画）。
- 窗口规则 `open-in-scratchpad` / `is-in-scratchpad`。
- 取消映射后再映射时保留暂存身份（v1：取消映射即退出暂存区）。
- 隐藏队列持久化（已定：不持久化）。
- 设置 xdg-toplevel `suspended`（已定：不设）。

### 1.3 设计原则

1. 配置关闭或不使用新动作时，行为与上游逐字节一致（测试快照不变）。
2. 复用现有机制：吸附端点、浮动位置/尺寸记忆、动态工作区维护、开窗动画。
3. 用户操作优先：手势、交互式 resize、拖动进行中时，自动视图移动一律让路。
4. 结构一致性靠公开操作自身维护，`refresh()` 只作兜底；`verify_invariants()` 断言最终状态。

---

## 2. 已定事项总表

| # | 事项 | 决策 |
|---|---|---|
| D1 | 隐藏窗口时窗口录屏 | 停止该窗口目标的录屏（`stop_casts_for_target`），仅在隐藏成功时执行 |
| D2 | Alt-Tab 是否列出隐藏窗口 | 不列（MRU 构建天然排除）；MRU 打开时隐藏的窗口要立即移除其缩略图 |
| D3 | 隐藏/呈现动画 | **修订**：v1 隐藏无动画；呈现用现有 window-open 动画；专门动画推迟到 M5 |
| D4 | 功能一在非活动工作区触发 | **修订**：总是用 horizontal-view-movement 动画（与上游一致） |
| D5 | 跟随范围 | 仅同一显示器 |
| D6 | suspended | 不设。理由：兼容性与范围（不是“隐藏后进程会冻结”） |
| D7 | 切换动作找不到候选 | 静默 + `debug!` |
| D8 | 显示中的窗口被拖到别处 | 落点是某显示器活动工作区的浮动层时，保留暂存身份；其余情况退出暂存区（§4.5.5） |
| D9 | 隐藏队列持久化 | 不保留 |
| D10 | 子窗口策略 | 有可见子窗口时拒绝隐藏；隐藏窗口出现新子窗口时先呈现父窗口（§4.6） |
| D11 | 取消映射后再映射 | 取消映射即退出暂存区；再映射按普通新窗口处理（v1） |
| D12 | 首次暂存尺寸 | 历史浮动尺寸 → 窗口规则 `default-column-width/default-window-height` → 工作区 60%×60%（§4.5.1） |
| D13 | `scratchpad-show` 同时给 `--id` 和 `--app-id` | 视为错误，IPC 返回错误信息 |
| D14 | `focus-window-previous` 是否会选中隐藏窗口 | 不会：候选中排除隐藏窗口（§4.8.1，M2b 后确认） |
| D15 | `set-dynamic-cast-window --id` 指向隐藏窗口 | 在 niri 层拒绝，与 id 不存在时相同（静默无操作），与 D1 一致（§4.8.1，M2b 后确认） |
| D16 | portal 窗口选择列表（gnome-shell introspect `GetWindows`）是否列出隐藏窗口 | 不列：在 niri 层排除，与 D1、D15 一致；想录制时先呈现（§4.8.1，M3 开始时确认） |

---

## 3. 功能一：移除后收紧视口

### 3.1 行为规格

一句话：平铺窗口离开滚动长带后，如果视图露出了“首末列吸附端点”之外的空白，就用一次动画把视图滑回端点；其余行为不变。本功能**不会**拉伸窗口铺满屏幕：总宽小于屏幕时，空白仍留在右侧。

名词：

- 视图位置 `p` = `target_view_pos()`。
- `L` / `R`：手势吸附的首列左端点 / 末列右端点（由 `view_offset_gesture_end()` 现有逻辑得出，包含 struts、最大化、全屏、on-overflow 的处理）。它们**不是**严格的全局合法区间（代码注释承认小尺寸固定宽度的最大化窗口加奇特 strut 时存在例外），只用作夹紧目标。
- 放得下：`L >= R`。

夹紧规则：

```
放得下：p' = L
否则：  p' = clamp(p, L, R)   // 仅在 L < R 时调用 clamp（f64::clamp 在 min > max 时 panic）
```

不另设阈值：交给 `animate_view_offset_with_config()` 内已有的“小于 1 物理像素不动”判断（`scrolling.rs` 约 734 行）。

触发（只看“真的从平铺层移走了 Tile/列”）：

| 场景 | 触发 | 落点 |
|---|---|---|
| 平铺窗口关闭 / 取消映射 | 是 | `Layout::remove_window` |
| 平铺窗口/列移到其他工作区或显示器 | 是，作用于源工作区 | 见 §3.4 表格 |
| 平铺 → 浮动（含 unfullscreen/unmaximize 回到浮动） | 是 | `Workspace::toggle_window_floating` |
| 平铺窗口送入暂存区 | 是 | §4.5.1 |
| 鼠标拖动开始（窗口脱离长带） | **否** | `remove_window_impl(.., false)` |
| 拖动结束（源是平铺） | 是，作用于源工作区，在窗口放回之后 | F1-5 |
| 合并进列导致列数减少 | 是 | F1-6 |
| 浮动窗口关闭/移走 | **否** | `!removed.is_floating` |
| 拆出列、改列宽、切换预设宽度 | 否 | 用户主动调整，不在本功能范围 |

不触发的状态（在 `fill_view_after_removal()` 内统一判断）：

1. 配置关闭。
2. 长带为空。
3. `view_offset.is_gesture()`（触控板、鼠标拖视图、DnD 边缘滚动）。
4. `interactive_resize.is_some()`。
5. 活动列 `sizing_mode()` **或** `pending_sizing_mode()` 不是 normal。
6. 居中模式：`is_centering_focused_column()` 为真（`center-focused-column "always"`，或只剩一列且开了 `always-center-single-column`）。

动画：总是使用 `animations.horizontal_view_movement`，与移除同一时刻发起。关窗快照存于长带坐标（`start_close_animation_for_window()` 约 1515 行 `tile_pos.x += self.view_pos()`），会随长带移动，不会错位。

已知的视觉细节：列移动用 `window-movement` 动画，视图移动用 `horizontal-view-movement` 动画。两者默认是相同的弹簧参数（`animations.rs` 约 260/276 行）。用户若把两者配成不同值，“视图在末端时关掉中间一列”会让右侧列在屏幕上轻微晃动。上游同类路径也有这个现象，本轮不处理。

### 3.2 F1-1/F1-2：配置与纯重构

**F1-1 配置**（`niri-config/src/layout.rs`）：

- `Layout` 加 `pub fill_view_after_removal: bool`，`Default` 中为 `false`。
- `LayoutPart` 加 `#[knuffel(child)] pub fill_view_after_removal: Option<Flag>`。
- 合并逻辑照抄 `always_center_single_column` 的写法（约 21、44、68、118 行）。这样自动支持按显示器、按工作区覆盖。
- 若 niri-config 的解析快照测试因新字段失败，用 `cargo insta review` 更新。
- 文档：`docs/wiki/Configuration:-Layout.md` 加一节，写明“不铺满、只消除超出首末列的空白”。

**F1-2 提取吸附端点**（`src/layout/scrolling.rs`，纯重构，单独提交）：

- 把 `view_offset_gesture_end()` 非居中分支里的 `snap_points` 闭包（约 3283–3337 行）提成私有方法：
  `fn column_snap_points(&self, col_x: f64, col: &Column<W>, prev_col_w: Option<f64>, next_col_w: Option<f64>) -> (f64, f64)`
- 把 `leftmost_snap / rightmost_snap` 的计算（约 3353–3373 行）提成：

```rust
/// 首列左吸附端点与末列右吸附端点（视图位置）。
/// 居中模式或无列时返回 None。返回值可能 left > right（所有列放得下）。
/// 注意：这不是严格的全局合法区间，见 view_offset_gesture_end 中的注释。
fn edge_snaps(&self) -> Option<(f64, f64)>
```

- `view_offset_gesture_end()` 改为调用这两个方法，行为不变。
- 验收：`cargo test` 全绿，现有 insta 快照零变化。

### 3.3 F1-3：夹紧函数

```rust
// scrolling.rs
pub fn fill_view_after_removal(&mut self) {
    if !self.options.layout.fill_view_after_removal {
        return;
    }
    if self.columns.is_empty()
        || self.view_offset.is_gesture()
        || self.interactive_resize.is_some()
    {
        return;
    }
    let col = &self.columns[self.active_column_idx];
    if !col.sizing_mode().is_normal() || !col.pending_sizing_mode().is_normal() {
        return;
    }
    let Some((left, right)) = self.edge_snaps() else {
        return;
    };

    let target = self.target_view_pos();
    let new_pos = if left >= right { left } else { target.clamp(left, right) };
    let new_offset = new_pos - self.column_x(self.active_column_idx);
    self.animate_view_offset_with_config(
        self.active_column_idx,
        new_offset,
        self.options.animations.horizontal_view_movement.0,
    );
}

// workspace.rs
pub fn fill_view_after_removal(&mut self) {
    self.scrolling.fill_view_after_removal();
}
```

要点：

- 用 `target_view_pos()`：移除前视图若在动画中，夹紧的是动画终点，`animate_view_offset_with_config()` 会从当前值平滑过渡。
- 第一个参数传活动列下标，函数内 `offset_delta` 为 0。
- 焦点列可见性：夹到 R 时末列右对齐，活动列只会往屏幕内移；夹到 L 对称。宽于视图的列无法“完全可见”，此时以左对齐为准（端点本身就是这样算的）。
- `activate_prev_column_on_removal` 不需要清：其恢复发生在之后的移除里，恢复后同一次移除会再夹紧一次。

### 3.4 F1-4：调用点

不改 `remove_tile()` 本身。在以下调用点紧跟一次调用：

| 文件 · 函数 | 约行 | 调用条件 |
|---|---|---|
| `mod.rs` `Layout::remove_window_impl`（Normal 分支） | 1200 | `fill_view && !removed.is_floating`，在清理空工作区之前 |
| 同上（NoOutputs 分支） | 1234 | 同上 |
| `mod.rs` `Layout::move_to_output` | 3387 | `!removed.is_floating`，作用于源工作区 |
| `mod.rs` `Layout::move_column_to_output` | 3441 | 总是（列只来自平铺层） |
| `monitor.rs` `Monitor::move_to_workspace` | 859 | `!removed.is_floating` |
| `monitor.rs` `Monitor::move_column_to_workspace` | 948 | 总是 |
| `workspace.rs` `Workspace::toggle_window_floating` 平铺→浮动分支 | 1445 | 总是（`set_window_floating` 和 unfullscreen/unmaximize 回浮动都经过这里） |

`Layout::remove_window` 拆分：

```rust
pub fn remove_window(&mut self, window: &W::Id, transaction: Transaction) -> Option<RemovedTile<W>> {
    self.remove_window_impl(window, transaction, true)
}
fn remove_window_impl(&mut self, window: &W::Id, transaction: Transaction, fill_view: bool)
    -> Option<RemovedTile<W>> { /* 原函数体 + 暂存区分支（§4.5.4） */ }
```

`interactive_move_update()`（约 3999 行，Starting→Moving 时）改调 `remove_window_impl(window, Transaction::new(), false)`。（就算漏改，非浮动拖动在 begin 时已对所有工作区调用 `dnd_scroll_gesture_begin()`，`is_gesture()` 也能兜底；但仍要显式排除。）

收尾检查：`grep -n "remove_tile(\|remove_active_column(\|remove_column_by_idx(" src/layout/*.rs`，逐个确认没有遗漏、也没有多余触发。`scrolling.rs` 内部的 consume/expel/swap 由 F1-6 单独处理。

### 3.5 F1-5 / F1-6：拖走与合并（M1 后半，可独立提交）

**F1-5 拖动结束后收紧源工作区**

- 为什么不放在 `dnd_scroll_gesture_end()`：`interactive_move_end()` 先结束所有工作区的 DnD 手势（约 4167–4174 行），之后才把窗口放回。如果在手势结束时夹紧，拖回同一工作区会先夹紧、再插入，视图滑两次。
- 做法：
  1. `InteractiveMoveData` 新增 `source_workspace: Option<WorkspaceId>` 和 `source_was_tiled: bool`。在 `interactive_move_update()` 的 Starting→Moving 分支里，在 `remove_window_impl` 之前记录（找到含该窗口的工作区 id；`source_was_tiled = !is_floating`）。**注意**：`is_floating` 必须取该分支开头、`set_fullscreen(false)/set_maximized(false)` **之前**的值——原本浮动的全屏窗口在取消全屏时会被立即移回浮动层，此时 `RemovedTile::is_floating` 为 true，但平铺层确实少了一列（测试 `interactive_move_of_floating_window` 第二个场景）。
  2. `interactive_move_end()` 的 Moving 路径在窗口放置**之后**、函数返回前：若 `source_was_tiled`，按 id 找到源工作区（可能已被清理，找不到就跳过），调用 `ws.fill_view_after_removal()`。拖回同一工作区时，此时长带已完整，调用是无害的。
  3. `remove_window_impl` 中“拖动中窗口被关闭”的分支（约 1167 行）：在 `dnd_scroll_gesture_end()` 之后，同样对源工作区调用一次（条件为 `fill_view && source_was_tiled`；该分支实际调用时 `fill_view` 总为 true，加上只为与参数语义一致）。
  4. 已知限制：拖动期间所有工作区的视图都被 DnD 手势锁住，松手后只补夹紧**源工作区**。拖动期间其他工作区发生的移除（如某窗口自行关闭）留下的空白不会被消除（见 §8）。

**F1-6 合并进列**

在 `Workspace` 转发 `consume_into_column`、`consume_or_expel_window_left/right` 的方法里包一层：调用前记录 `scrolling` 列数，调用后若列数减少，就调用 `self.scrolling.fill_view_after_removal()`。

### 3.6 功能一测试

新建 `src/layout/tests/fill_view.rs`（仿 `tests/animations.rs` 的 insta 风格），并在 `tests.rs` 的 options 生成器里让 `fill_view_after_removal` 随机取 true/false，扩大 `random_operations_dont_panic` 的覆盖。

| # | 场景 | 断言 |
|---|---|---|
| 1 | 关最右列，左邻列原本可见 | `CompleteAnimations` 后 `view_pos == R`，活动列可见 |
| 2 | 视图在末端，关中间列 | 同上 |
| 3 | 关活动列左侧的列，左侧露白 | `view_pos == L` |
| 4 | 剩余列总宽小于屏幕 | `view_pos == L` |
| 5 | 只剩一列且开了 `always-center-single-column`，或 `center-focused-column "always"` | 与开关关闭时位置相同 |
| 6 | 关最后一个窗口 | 无 panic，无变化 |
| 7 | 新开窗口后立刻关掉（`activate_prev_column_on_removal`） | 结果无留白；原位置本就无白时与上游相同 |
| 8 | 关窗时视图正在动画 | 终点被夹紧 |
| 9 | 手势中关窗 | **差分**：开关开/关时手势期间视图相同；`ViewOffsetGestureEnd` 后不断言区间（`L > R` 时可能吸附到 R） |
| 10 | 拖动：`InteractiveMoveBegin` + `InteractiveMoveUpdate`（越过阈值） | 源工作区视图在拖动中不变；`InteractiveMoveEnd` 落到别的工作区后，源工作区被夹紧（F1-5） |
| 11 | 活动列全屏/最大化，或已请求退出全屏尚未提交（pending normal、实际 fullscreen） | 不夹紧 |
| 12 | 关闭/移走**浮动**窗口，平铺层原本有留白 | 平铺视图不动 |
| 13 | 非活动工作区关窗 | 动画终点被夹紧（`CompleteAnimations` 后断言） |
| 14 | 交互式 resize 进行中关掉另一列 | 不夹紧 |
| 15 | 有 struts、gaps 0/16、宽于工作区的列 | 结果等于 `edge_snaps()` 的端点 |
| 16 | 平铺→浮动、移到其他工作区/显示器 | 源工作区被夹紧 |
| 17 | `ConsumeWindowIntoColumn` 使末列消失 | 被夹紧（F1-6） |
| 18 | 开关关闭 | 所有现有快照不变（回归保护） |

手动矩阵（嵌套 niri）：gaps 0/16 × 缩放 1/1.25/2 × center-focused-column never/on-overflow/always × 有无左右 bar。观察：一次到位、无两段式；关窗快照跟随长带；快速连续关窗稳定；Overview 中关窗无跳变。

---

## 4. 功能二：全局暂存区

### 4.1 概念与状态

暂存区是窗口的一种**状态**，不是一个工作区。

| 状态 | 窗口在哪 | 渲染 | IPC `workspace_id` | IPC `is_in_scratchpad` | IPC `is_scratchpad_hidden` |
|---|---|---|---|---|---|
| None（普通） | 任意工作区的平铺/浮动层，或拖动中 | 是 | 有（拖动中为 null） | false | false |
| Hidden | `Layout.scratchpad.hidden` 队列 | 否 | null | true | true |
| Shown | 某显示器**活动工作区**的浮动层（或拖动中） | 是 | 有（拖动中为 null） | true | false |

`is_scratchpad_hidden` 必须显式提供：拖动中的窗口 `workspace_id` 也是 null，不能靠 `workspace_id == None` 推断（R16 建议的 `is_hidden`）。

### 4.2 状态转换总表

| 从 → 到 | 触发 |
|---|---|
| None → Hidden | `move-window-to-scratchpad`（含 `--id`） |
| Shown → Hidden | 同上；或 `scratchpad-show` 作用于已聚焦的显示窗口 |
| Hidden → Shown | `scratchpad-show`（通用/选择器/`--id`）；`Layout::activate_window`（xdg-activation 经规则放行、任务栏、`focus-window --id`）；隐藏窗口出现新子窗口（§4.6） |
| Hidden → None | `remove-window-from-scratchpad`：先在当前活动工作区呈现，再清标记；无输出时不做任何事 |
| Shown → None | `remove-window-from-scratchpad`；切到平铺（含全屏、最大化，均经 `toggle_window_floating`）；经 `Workspace::add_tile` 加入任何工作区（移到其他工作区/显示器、拖动落点不满足 D8） |
| Shown → Shown（换工作区） | 跟随（§4.7）；`scratchpad-show` 规则 3 拉到当前显示器 |
| Hidden → 销毁 | 程序退出、取消映射、`close-window --id` 后程序退出 |

**规则**：任何把窗口放入平铺层或明确移走的操作，都会让窗口退出暂存区。清标记发生在转换**真正发生**的位置（`Workspace::add_tile` 入口、`toggle_window_floating` 的浮动→平铺分支），所以无效目标、重复请求等空操作不会误清。

### 4.3 动作语义（精确定义）

候选集 = 所有 Hidden 或 Shown 状态、且通过过滤器的窗口。过滤器：通用 = 全部；`--id N` = 仅该窗口；`app-id=RE` = app-id 匹配 RE 的窗口（`is_match`，与窗口规则语义相同）。

**`scratchpad-show [app-id=RE | --id N]`**（`Layout::toggle_scratchpad(filter)`），按顺序判断，命中即停：

1. 当前聚焦窗口 F 处于 Shown 且通过过滤器 → **隐藏 F**（走 §4.5.1，可能因子窗口被拒绝，此时什么都不做）。返回 `Hidden(id)`；被拒绝时返回 `Nothing`。
2. 聚焦显示器的活动工作区上，有 Shown 状态的候选 → 聚焦其中 `scratchpad_shown_seq` 最大的那个。返回 `Focused(id)`。
3. 聚焦显示器活动工作区**以外**有 Shown 状态的候选（其他显示器；或 M4 跟随实现之前，同一显示器的非活动工作区）→ 取 `scratchpad_shown_seq` 最大者：
   - 没有可见子窗口：从原工作区移除（`Workspace::remove_tile`，会 `output_leave`），赋予新的 `scratchpad_shown_seq`（视为一次呈现），加入当前活动工作区浮动层并聚焦（`add_scratchpad_tile(.., enter_output = true)`，位置按比例恢复），然后 `ensure_empty_workspaces_after_add`；若原工作区因此变空且不是其显示器的活动工作区，就走 §4.5.1 第 11 步的清理辅助函数。返回 `Shown(id)`；
   - 有可见子窗口：改为 `activate_window(id)`（焦点切到它所在的显示器/工作区），不搬动。返回 `Focused(id)`。
4. 有 Hidden 状态的候选 → 取隐藏队列中第一个匹配者（FIFO），呈现并聚焦。返回 `Shown(id)`。
5. 否则什么都不做，打 `debug!`，返回 `Nothing`。

无输出（`MonitorSet::NoOutputs`）时没有聚焦的显示器，五条规则都不适用，直接返回 `Nothing`。

由此得到的性质（写入用户文档）：

- 只用通用键、且开始时全部隐藏：每次只显示一个，并形成 A→B→C→A 轮换（隐藏时排到队尾，呈现时从队首取）。
- 已有专属键呈现的窗口时，通用键会先聚焦它（规则 2），不会立刻呈现新的隐藏窗口。**不保证同一时刻只显示一个。**
- 带选择器时只在匹配集合内取队首，不打乱其他窗口在队列中的顺序。

**`move-window-to-scratchpad [--id N]`**（`Layout::move_to_scratchpad`）：None 或 Shown → Hidden；Hidden → 无操作；拖动中（Starting 或 Moving）→ 无操作；有可见子窗口 → 无操作 + `debug!`。

**`remove-window-from-scratchpad [--id N]`**（`Layout::remove_from_scratchpad`）：Shown → None（窗口保持为当前位置的普通浮动窗口）；Hidden → 在聚焦显示器的活动工作区呈现并聚焦，然后清标记；Hidden 且无输出 → 无操作；None → 无操作。拖动中的 Shown 窗口：清标记即可。Shown 窗口在无输出工作区中（移除全部输出后留在原工作区浮动层，只能用 `--id` 指定）→ 照常清标记。

**激活（`Layout::activate_window(id)`）**：Hidden → 呈现并聚焦（§4.5.3），**永不隐藏**；Shown/None → 原逻辑。`activate_window_without_raising(id)`：Hidden → 无操作。

### 4.4 焦点规则

- 呈现：`activate=true` 时被呈现的窗口获得焦点，浮动层成为活动层。
- 隐藏：交给现有逻辑。`Workspace::remove_tile()` → `update_focus_floating_tiling_after_removing()`：浮动层还有窗口则聚焦最上层的，否则回到平铺层活动列。
- 跟随：只保留原工作区内的焦点关系（§4.7）。暂存窗口原本是该工作区的焦点窗口，跟随后仍是；否则跟随后不抢焦点。
- 键盘焦点、弹出菜单 grab：由 `Niri::update_keyboard_focus` 根据 `layout.focus()` 重算；焦点离开 grab 根时现有代码会 ungrab（`niri.rs` 约 1436 行）。

### 4.5 Layout 层实现

#### 4.5.0 数据结构

```rust
// src/layout/mod.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScratchpadState { #[default] None, Hidden, Shown }

struct Scratchpad<W: LayoutElement> {
    /// FIFO：隐藏时 push_back；呈现时从前往后找第一个匹配。
    hidden: VecDeque<Tile<W>>,
    /// 递增序号，写入 Tile.scratchpad_shown_seq，用于“最近呈现”。
    next_seq: u64,
}
// Layout 新增字段：scratchpad: Scratchpad<W>

// src/layout/tile.rs —— Tile 新增
pub(super) in_scratchpad: bool,        // Hidden 或 Shown 时为 true
pub(super) scratchpad_shown_seq: u64,

/// 清空全部动画：open / resize / move_x / move_y / alpha（含 hold_after_done）。
pub fn stop_all_animations(&mut self);
```

只存 `Tile`：它带有 `floating_pos`（按工作区比例）、`floating_window_size`（**逻辑像素，不是比例**）、预设宽高下标。重新加入浮动层不需要 `ColumnWidth`。

**同步规则**：Tile 标记与窗口状态只能经由一个辅助函数修改：

```rust
fn set_tile_scratchpad<W: LayoutElement>(tile: &mut Tile<W>, state: ScratchpadState) {
    tile.in_scratchpad = state != ScratchpadState::None;
    tile.window_mut().set_scratchpad_state(state);
}
```

`LayoutElement` 新增 `fn set_scratchpad_state(&mut self, state: ScratchpadState);` 和 `fn scratchpad_state(&self) -> ScratchpadState;`（读取方法写法同 `sizing_mode()` / `is_urgent()`；`verify_invariants()` 是泛型代码，只能经 trait 读取窗口状态）。

- `src/window/mapped.rs`：`set_scratchpad_state` 照抄 `set_floating()` 的写法（约 998 行）：存字段，状态变化时置 `need_to_recompute_rules`；`scratchpad_state` 返回该字段（niri 层经 trait 调用，不再另设同名固有方法）。
- 测试的 `TestWindow`：用 `Cell` 记录状态，供不变量断言。

#### 4.5.1 隐藏：`Layout::move_to_scratchpad(&mut self, window: Option<&W::Id>) -> Option<W::Id>`

返回被隐藏窗口的 id，供 niri 层执行副作用（§4.8）。步骤：

1. `id = window` 或 `self.focus()` 的 id；都没有则返回 `None`。
2. 拖动状态（Starting 或 Moving）涉及该 id → `None`。
3. 已是 Hidden → `None`。
4. **子窗口检查**：存在任何非 Hidden 窗口 `w` 满足 `w.is_child_of(target)`（遍历所有工作区和拖动中的窗口）→ `debug!` 后返回 `None`。直接遍历工作区和拖动中的 Tile，不经过 `windows()`：M2b 让 `windows()` 包含隐藏窗口后，这里仍只看非隐藏窗口。
5. 找到含该窗口的工作区（Normal：显示器/工作区下标；NoOutputs：工作区下标）。
6. 先 `ws.set_fullscreen(&id, false); ws.set_maximized(&id, false);`，与 `interactive_move_update()` 的做法一致。这可能经 `toggle_window_floating` 把窗口挪回浮动层（同时触发功能一）。
7. `let removed = ws.remove_tile(&id, Transaction::new());`（内部会 `output_leave`、更新焦点、浮动层还会记录尺寸和位置）。
8. 若 `!removed.is_floating`：
   - 首次尺寸（D12）：若 `removed.tile.floating_window_size.is_none()`：

     ```rust
     let rules = tile.window().rules();
     let w = match rules.default_width  { Some(Some(w)) => w, _ => PresetSize::Proportion(0.6) };
     let h = match rules.default_height { Some(Some(h)) => h, _ => PresetSize::Proportion(0.6) };
     let win = tile.window();
     let size = ws.new_window_size(Some(w), Some(h), true, rules, (win.min_size(), win.max_size()));
     tile.floating_window_size = Some(size);
     ```

   - `floating_pos` 保持原值（曾经浮动过就回到旧位置；为 None 时，呈现时由 `default-floating-position` 规则或居中决定）。
   - `ws.fill_view_after_removal()`（功能一）。
9. 处理 Tile：`stop_all_animations()`；`interactive_move_offset = (0,0)`；`take_unmap_snapshot()` 丢弃；`set_tile_scratchpad(Hidden)`；`window_mut().set_activated(false)`；`window_mut().set_floating(true)`。
10. `hidden.push_back(tile)`。
11. 清理空工作区：把 `remove_window` 中的那段（约 1202–1226 行：非活动、非末尾、无切换动画的空工作区移除，以及 `empty_workspace_above_first` 特例）提取成辅助函数，两处共用。NoOutputs 分支同理。
12. 返回 `Some(id)`。

不再先 `set_window_floating`：既避免平铺尺寸被当作浮动尺寸，也避免 M5 的收起动画从错误位置开始。

#### 4.5.2 呈现（内部）：`fn scratchpad_show(&mut self, id: &W::Id, activate: bool) -> bool`

1. 不是 `MonitorSet::Normal` → `false`。
2. 在 `hidden` 中找到该 id 并取出，找不到 → `false`。
3. 目标：活动显示器的活动工作区 `ws_idx`。
4. `set_tile_scratchpad(Shown)`；`tile.scratchpad_shown_seq = next_seq; next_seq += 1`。
5. `ws.add_scratchpad_tile(tile, activate, /*enter_output*/ true)`（新增于 workspace.rs）：
   - `enter_output` 为真时调用 `enter_output_for_window()`（设置首选缩放/变换并 `output_enter`）；
   - `self.floating.add_tile(tile, activate)`：内部 `add_tile_at()` 会 `update_config(view_size, scale, options)`，按 `floating_window_size` 请求尺寸，按 `stored_or_default_tile_pos()` 定位；
   - `if activate || self.scrolling.is_empty() { self.floating_is_active = FloatingActive::Yes; }`；
   - **不**清标记（区别于 `add_tile`）。
6. 开窗动画：添加后对该 Tile 调用 `start_open_animation()`。动画关闭时 Animation 即刻完成。
7. `mon.ensure_empty_workspaces_after_add(ws_idx)`：从 `Monitor::add_tile()` 末尾（约 605–614 行）提取——若 `ws_idx` 是最后一个就 `add_workspace_bottom()`；若 `empty_workspace_above_first && ws_idx == 0` 就 `add_workspace_top()`。签名 `fn ensure_empty_workspaces_after_add(&mut self, idx: usize) -> usize`，返回调整后的下标（`add_workspace_top()` 之后为 `idx + 1`），原 `add_tile` 用它激活工作区。原 `add_tile` 改为调用它；`Monitor::add_column()` 中相同的代码本轮不动。不修改 `original_outputs`。
8. 返回 `true`。

跨显示器尺寸（R4）：位置按比例恢复；浮动层的位置校正只保证窗口部分可见，**不会**自动缩小过大的窗口。v1 不承诺完全落在屏内，只承诺可找回、可移动。

#### 4.5.3 公开接口

```rust
pub enum ScratchpadToggle<Id> { Hidden(Id), Focused(Id), Shown(Id), Nothing }

pub fn move_to_scratchpad(&mut self, window: Option<&W::Id>) -> Option<W::Id>;
pub fn toggle_scratchpad(&mut self, filter: impl Fn(&W) -> bool) -> ScratchpadToggle<W::Id>;
pub fn remove_from_scratchpad(&mut self, window: Option<&W::Id>);
pub fn is_scratchpad_hidden(&self, window: &W::Id) -> bool;
pub fn scratchpad_hidden_windows(&self) -> impl Iterator<Item = &W>;
```

- 过滤器是泛型谓词，`Layout<W>` 不需要知道 app-id；app-id 匹配在 niri 层的闭包里完成（R 六.3）。
- `toggle_scratchpad` 实现 §4.3 的 5 条规则。规则 1 返回 `Hidden(id)`（niri 层据此执行副作用）。
- `activate_window()`：在现有 interactive_move 检查之后加入

  ```rust
  if self.is_scratchpad_hidden(window) { self.scratchpad_show(window, true); return; }
  ```

- `activate_window_without_raising()`：Hidden → `return`。

#### 4.5.4 生命周期分支

- `remove_window_impl()`：在 interactive_move 分支之后、遍历工作区之前，先查隐藏队列。找到就取出，返回 `RemovedTile { tile, width: ColumnWidth::Fixed(tile.tile_size().w), is_full_width: false, is_floating: true }`；不触发功能一，也不需要清理工作区。
- `start_close_animation_for_window()`、`store_unmap_snapshot()`、`clear_unmap_snapshot()`：对隐藏窗口直接返回（关闭无动画）。**现有实现已自然满足**：三者都直接遍历工作区查找窗口，找不到隐藏窗口即什么都不做，因此不加冗余检查。M2b 修改 A 类方法时，不要把它们改成经过 `windows()` / `has_window()` 查找。
- 取消映射（`compositor.rs` 约 262–292 行）沿用现有流程：`stop_casts_for_target` → `window_mru_ui.remove_window` → `layout.remove_window`。隐藏窗口因此退出暂存区（D11）。

#### 4.5.5 退出暂存区的落点

1. `Workspace::add_tile()` 入口：`if tile.in_scratchpad { set_tile_scratchpad(&mut tile, None) }`。覆盖：移到其他工作区/显示器、拖动落点、新窗口。
2. `Workspace::add_tile_to_column()` 入口：同上（拖进某列）。
3. `Workspace::toggle_window_floating()` 浮动→平铺分支：在 `scrolling.add_tile` 前清标记。覆盖：切平铺、全屏、最大化（`set_fullscreen`/`set_maximized` 对浮动窗口都调用 `toggle_window_floating`，见 workspace.rs 约 1288/1355 行）。
4. 拖动（D8）：`InteractiveMoveData` 在拖动期间持有原 Tile（标记保持 Shown）。`interactive_move_end()` 在放置前记录 `let was_scratchpad = move_.tile.in_scratchpad;`。放置会因第 1/2 条清掉标记。放置后，若 `was_scratchpad`，且窗口最终位于**某显示器活动工作区的浮动层**，就调用 `ws.restore_scratchpad_shown(&id)`（新增：`set_tile_scratchpad(Shown)`，seq 不变）。其余情况（落到平铺、Overview 中落到非活动工作区）保持 None。
5. `remove_from_scratchpad`：`Workspace::clear_scratchpad(&id)`，或对拖动中的 Tile 直接清。

#### 4.5.6 refresh / update / 动画 / 配置

- `Layout::refresh()`：在 interactive_move 块（约 4877–4890 行）之后，对每个隐藏 Tile 执行：

  ```rust
  let win = tile.window_mut();
  win.set_active_in_column(true);
  win.set_floating(true);
  win.set_activated(false);
  win.set_interactive_resize(None);
  win.send_pending_configure();
  win.refresh();
  ```

  `set_bounds` 保留上次的值，不更新。refresh 末尾再对所有显示器调用一次 `scratchpad_follow()` 作兜底（§4.7）。
- `Layout::update_window()`：在 interactive_move 分支之后加隐藏分支。先 `on_commit(serial)`（若有），再 `tile.update_window()`，最后 `tile.stop_all_animations()`，与拖动分支写法一致。
- `advance_animations()` / `are_animations_ongoing()`：不包含隐藏 Tile（它们没有动画）。`verify_invariants` 断言隐藏 Tile `!are_transitions_ongoing()`。
- `update_shaders()`：对隐藏 Tile 调用 `tile.update_shaders()`。
- `update_config()`：不需要处理隐藏 Tile（呈现时 `add_tile_at` 会 `update_config`）。
- `descendants_added()`：隐藏窗口 → `false`。现有实现只遍历工作区，已自然满足，不加代码（同 §4.5.4 的说明）。
- refresh 末尾的 `scratchpad_follow()` 兜底属于 M4，M2 不做。

#### 4.5.7 现有方法审计（必须逐个过）

`grep -n "InteractiveMoveState::Moving(move_)) = &" src/layout/mod.rs`：凡是带这个前置检查、参数含窗口 id 的方法，都必须在旁边加隐藏窗口的处理。分类如下：

| 类别 | 方法 | 对隐藏窗口 |
|---|---|---|
| A 必须能找到 | `windows()`（monitor 为 None）、`with_windows()`（output None、workspace None，layout 用 `ipc_layout_template()`）、`with_windows_mut()`（output None）、`find_window_and_output(_mut)()`（output None）、`has_window()`、`update_window()` | 包含隐藏 Tile。注意：预提交钩子（`xdg_shell.rs` 约 1498 行）、提交处理（`compositor.rs` 约 262 行）、窗口规则重算（`niri.rs` 约 4186、6947 行）、tiled state（约 4172 行）、兜底帧回调（约 5286 行）、foreign-toplevel、IPC、录屏属性都依赖 A 类 |
| B 必须看不到 | `windows_for_output(_mut)()`、各 render、`window_under()`、`focus()`/`focus_with_output()`、`workspaces()` | 不加入，自然排除 |
| C 单窗口操作（可传 id） | `popup_target_rect()`（返回零矩形）、`scroll_amount_to_activate()`（0）、`should_trigger_focus_follows_mouse_on()`（false）、`consume_or_expel_window_left/right()`、`center_window()`、`toggle_window_width/height()`、`set_window_width/height()`、`reset_window_height()`、`toggle_window_floating()`、`set_window_floating()`、`move_floating_window()`、`interactive_move_begin()`（false）、`interactive_resize_begin()`（false）、`interactive_resize_end()` | id 是隐藏窗口时直接返回（注意多数方法内部是 `find(..).unwrap()`，不加检查会 panic） |
| C' 状态请求 | `set_fullscreen()`、`toggle_fullscreen()`、`toggle_windowed_fullscreen()`、`set_maximized()`、`toggle_maximized()` | 隐藏中忽略 |
| C'' 移走 | `move_to_workspace()`、`move_to_output()`、`move_column_to_*`（按 id 的变体） | 隐藏中直接返回 |
| C''' 激活 | `activate_window()` / `activate_window_without_raising()` | §4.5.3 |
| C 自然满足（不加代码） | `interactive_move_update()`、`interactive_move_end()`（只在 Starting/Moving 的 id 匹配时动作，而 Starting/Moving 不可能是隐藏窗口，`verify_invariants` 断言）、`interactive_resize_update()`（遍历工作区，返回 false）、`start_open_animation_for_window()`（遍历工作区，无操作） | 对隐藏窗口自然无操作，不加冗余检查（同 §4.5.4）；M2b 审计时补入本表 |
| D 生命周期 | §4.5.4、§4.5.6 | |
| E 新窗口 | `add_window()` 的 `NextTo` | §4.6 |

审计完成的标准：proptest 的 Op 里所有带窗口 id 的操作，都能随机命中隐藏窗口（`has_window()` 对隐藏窗口返回 true），`random_operations_dont_panic` 长时间运行不 panic。

### 4.6 子窗口策略（D10）

1. **隐藏时**：有任何非隐藏的直接子窗口（`is_child_of`）→ 拒绝（§4.5.1 第 4 步）。孙窗口必然伴随子窗口存在，所以只查直接子窗口即可。
2. **隐藏窗口出现新子窗口**：在 `Layout::add_window()` 开头（解构 `monitor_set` 之前）：

   ```rust
   if let AddWindowTarget::NextTo(next_to) = target {
       if self.is_scratchpad_hidden(next_to) {
           // 有输出：先按子窗口的激活意图呈现父窗口，再照常 NextTo。
           // 无输出：父窗口留在队列中，target 改为 Auto（否则 NoOutputs 分支的 NextTo 会 unwrap 失败）。
       }
   }
   ```

   父窗口的 `activate` 取 `!matches!(activate, ActivateWindow::No)`。这样 `Smart` 时，子窗口会因父窗口处于活动状态而被激活；`strict-new-window-focus-policy` 下（`No`），两者都不抢焦点。
   `compositor.rs` 约 178 行的父窗口过滤对隐藏父窗口（output 为 None）会放行，无需修改。
3. **运行时改父窗口**（`parent_changed` / `descendants_added`）：隐藏父窗口 → 无操作；不 panic。
4. 父链可能出现环（测试里的 `SetParent` 可以构造），所有父/子遍历只看直接关系，不做递归。
5. 已知限制：显示中的暂存窗口跟随工作区时，其对话框作为普通窗口不跟随。

### 4.7 同显示器跟随（D5）

**规则**：显示器 M 的任意非活动工作区中，浮动层里所有 Shown 状态的 Tile 都搬到 M 的活动工作区。

```rust
// monitor.rs
pub fn scratchpad_follow(&mut self) {
    // 1. 对每个 idx != active_workspace_idx 的工作区：
    //    ws.take_shown_scratchpad_tiles() -> Vec<(Tile<W>, bool /*是该工作区的焦点窗口*/)>
    //    按浮动层叠顺序从上到下取出；
    //    用 FloatingSpace 的移除（会记录 pos/size），不调用 output_leave；
    //    取完后调用 update_focus_floating_tiling_after_removing(true)。
    //    "是焦点窗口" = floating.active_window_id == id && floating_is_active。
    // 2. 若为空则返回。
    // 3. 按从下到上的顺序：active_ws.add_scratchpad_tile(tile, was_focused, /*enter_output*/ false)。
    // 4. self.ensure_empty_workspaces_after_add(active_workspace_idx)（可能使 active_workspace_idx +1）。
}
```

- 幂等、不调用 `activate_workspace*`，所以不会递归。
- 源工作区搬空后，由现有 `clean_up_workspaces()` 在切换动画结束时清理（`monitor.rs` 约 690 行）。
- 不修改 `original_outputs`。
- 只有暂存窗口的工作区算作普通非空工作区，这与 niri 的模型一致。副作用是：有显示中的暂存窗口时，切到底部空工作区会使其变为非空并新增一个空工作区。这是预期行为，写入用户文档。

**调用点**：

| 位置 | 说明 |
|---|---|
| `Monitor::activate_workspace_with_anim_config()` 末尾（含 `prev_active_idx == idx` 的提前返回路径；先改成单一出口） | 键盘/IPC/bar 切换、`switch_workspace`、`Monitor::add_tile` 的激活、DnD 切换 |
| `Monitor::workspace_switch_gesture_end()` 设置 `active_workspace_idx` 之后（约 2078 行） | 手势切换不经过上一个函数 |
| `Monitor::append_workspaces()` 末尾 | 输出插拔导致活动工作区变化 |
| `Layout::add_output()`、`Layout::remove_output()` 末尾，对所有显示器 | 工作区在显示器间迁移；被拔显示器上的显示窗口随工作区迁移后，搬到接收显示器的活动工作区 |
| `Layout::refresh()` 末尾，对所有显示器 | 兜底 |

实现者还需 `grep -n "active_workspace_idx = \|active_workspace_idx -= \|active_workspace_idx += " src/layout/{monitor,mod}.rs`，逐个判断是“重新编号（同一工作区）”还是“活动工作区真的变了”，后者补调用。由 `verify_invariants` 的第 4 条兜住遗漏（§4.11）。

工作区切换手势进行中：`active_workspace_idx` 在松手前不变，窗口留在旧工作区随之滑动；松手后再跟随。Overview 中切换工作区：窗口会在缩略工作区之间跳动，v1 接受。

### 4.8 niri 层：副作用、可见性、帧回调

**统一副作用入口**（`src/input/mod.rs` 或 `src/niri.rs` 新增 `State::on_window_hidden_to_scratchpad(&mut self, window: &Window)`），仅在 `Layout` 返回“已隐藏”时调用：

1. `self.niri.stop_casts_for_target(CastTarget::Window { id })`（D1）。
2. 立即清可见性缓存：对窗口全部表面（`window.with_surfaces`）、对每个输出调用 `PrimaryScanoutOutput::update_from_render_element_states(id, output, None, &RenderElementStates::default(), |_, _, o, _| o)`。smithay 在“未呈现且当前输出相同”时会把缓存置 None。这样 idle inhibit（`refresh_idle_inhibit`，`niri.rs` 约 4160 行）立即释放，不再按刷新率发帧回调。
3. 关闭该窗口的全部弹出菜单：`for (popup, _) in PopupManager::popups_for_surface(wl_surface) { let _ = PopupManager::dismiss_popup(wl_surface, &popup); }`（grab 菜单在焦点变化时也会被 ungrab，这里再显式关闭非 grab 菜单，避免再次呈现时出现位置过期的菜单）。**必须在第 2 步之后**：`dismiss_popup()` 会立即把弹出层从 PopupTree 中删除，而 `with_surfaces` 正是经 PopupTree 找弹出层的，先关菜单会漏清弹出层的缓存（陷阱 11；M3 实现时发现并调整顺序）。
4. `self.niri.window_mru_ui.remove_window(id)`（MRU 已打开时删除旧缩略图，D2）。
5. `queue_redraw_all()`。

**持续兜底**：`Niri::update_primary_scanout_output()` 的窗口循环（约 4956 行）改为遍历 `windows_for_output(output).chain(layout.scratchpad_hidden_windows())`。隐藏窗口从不被呈现，任何输出渲染时都会清掉缓存。

**帧回调说明（D6 的正确理由）**：隐藏窗口仍在 `with_windows_mut` 中，因此会收到 1 秒兜底定时器的节流回调（与非活动工作区窗口相同），客户端保持活性。逐输出的高频回调会因缓存被清而停止。**不承诺**程序后台的计算、音频、定时器会停止。

**呈现后**：不需要额外副作用；`queue_redraw_all()` 即可。

#### 4.8.1 niri 层排除隐藏窗口（D14、D15）

M2b 起 `Layout::windows()` 包含隐藏窗口（§4.5.7 A 类），niri 层少数按 `windows()` 选窗口的动作需要显式排除。判断用 `mapped.scratchpad_state() == ScratchpadState::Hidden`（需 `use crate::layout::LayoutElement`）。

| 位置（`src/input/mod.rs` `do_action()`） | 处理 |
|---|---|
| `Action::FocusWindowPrevious`（约 906 行） | 候选迭代器加 `.filter(|win| win.scratchpad_state() != ScratchpadState::Hidden)`。否则刚隐藏的窗口焦点时间戳最新，会被选中并经 `activate_window` 呈现（D14） |
| `Action::SetDynamicCastWindowById`（约 2273 行） | `any(..)` 条件加上“不是 Hidden”；隐藏窗口与 id 不存在时相同，静默无操作（D15）。`SetDynamicCastWindow`（无 id）作用于聚焦窗口，不可能是隐藏窗口，无需处理 |
| `src/niri.rs` `State::on_introspect_msg()`（约 2287 行，不在 `do_action()` 中） | `with_windows()` 闭包内跳过 Hidden 窗口（D16） |

不改的（已核对）：`close-window --id` 对隐藏窗口有效（§4.2）；`screenshot-window --id` 要求有 monitor，对隐藏窗口自然无操作；其余 `--id` 动作交给 Layout 的 C 类检查；MRU 与 a11y 的按 id 查找只用于读取。

**portal 窗口选择列表（D16，M3 开始时确认）**：列表来自 `with_windows()`（`niri.rs` 约 2308 行，gnome-shell introspect 的 `GetWindows`），M2b 后会列出隐藏窗口。核对源码：选中隐藏窗口时 `cast_params_for_window()` 经 `windows()` 能找到它，`refresh_mapped_cast_outputs()` 也保留其上次的输出，所以录屏能启动、但隐藏期间画面不更新，与 D1 的意图不一致。决定：列表中排除隐藏窗口。列表获取之后才隐藏的窗口由 D1 的停止录屏处理。

### 4.9 IPC

`niri-ipc/src/lib.rs`：

```rust
// Window 结构体新增
/// 窗口处于暂存区（隐藏或显示）。
#[serde(default)]
pub is_in_scratchpad: bool,
/// 窗口隐藏在暂存区中（此时 workspace_id 为 None）。
#[serde(default)]
pub is_scratchpad_hidden: bool,

// Action 新增（clap 用 #[cfg_attr(feature = "clap", arg(long))]）
MoveWindowToScratchpad { id: Option<u64> },
ScratchpadShow { id: Option<u64>, app_id: Option<String> },
RemoveWindowFromScratchpad { id: Option<u64> },
```

`src/ipc/server.rs`：

- `make_ipc_window()`（约 518 行）填充两个字段（来自 `mapped.scratchpad_state()`）。
- `ipc_refresh_windows()`（约 722 行）的 `changed` 判断**必须**加上

  ```rust
  || ipc_win.is_in_scratchpad != (state != ScratchpadState::None)
  || ipc_win.is_scratchpad_hidden != (state == ScratchpadState::Hidden)
  ```

  否则“Shown→None 且位置、浮动状态、标题都不变”时不会发事件。
- `validate_action()`（约 464 行）：`ScratchpadShow` 同时给出 `id` 和 `app_id` → `Err("id and app-id are mutually exclusive")`（D13）；`app_id` 正则无效 → `Err(format!("invalid app-id regex: {err}"))`。
- `niri msg windows` 的文本输出（`src/ipc/client.rs` 打印窗口处）加一行暂存状态（可选）。

兼容性：`#[serde(default)]` 只保证新客户端能读旧服务端的输出；旧服务端不认识新 Action，会报解析错误，这是预期的。

### 4.10 激活路径（R14）

不改 `handlers/mod.rs` 的 `request_activation()`（约 805 行）分支逻辑：

- `on-xdg-activate` 为 `Focus`、或无规则且不是 `UrgentOnlyMarker` → 调用 `layout.activate_window` → 隐藏窗口被呈现（§4.5.3）。
- `SetUrgent` / `UrgentOnlyMarker` → `mapped.set_urgent(true)`，对隐藏窗口同样生效，窗口保持隐藏，IPC `is_urgent` 变为 true。
- `Ignore` → 无变化。
- 令牌过期 → 现有代码直接忽略。

任务栏 `ForeignToplevelHandler::activate`（约 536 行）、`focus-window --id`（`State::focus_window` → `activate_window`）同样会呈现隐藏窗口。

锁屏期间：布局允许变化（与锁屏期间普通窗口激活一致），锁屏表面的输入与渲染边界不受影响。

### 4.11 不变量（加入 `Layout::verify_invariants()`）

1. 隐藏队列中每个 Tile：`in_scratchpad == true`，窗口状态 Hidden，`!are_transitions_ongoing()`，不出现在任何工作区，也不是拖动中的 Tile。
2. 任何工作区平铺层中的 Tile：`in_scratchpad == false`，状态 None。
3. 任何浮动层中的 Tile：`in_scratchpad` ⇔ 状态 Shown；状态从不为 Hidden。
4. `MonitorSet::Normal` 时：带 `in_scratchpad` 的浮动 Tile 只出现在其显示器的 `active_workspace_idx` 工作区。
5. 拖动中的 Tile：状态不为 Hidden。
6. 同一窗口 id 在隐藏队列、所有工作区、拖动中三者合计最多出现一次。
7. 现有不变量（末尾空工作区等）在任何操作后仍成立。

### 4.12 功能二测试

**proptest Op**（`src/layout/tests.rs`）：

```rust
MoveWindowToScratchpad { #[proptest(strategy = "proptest::option::of(1..=5usize)")] id: Option<usize> },
ToggleScratchpad      { #[proptest(strategy = "proptest::option::of(1..=5usize)")] id: Option<usize> }, // None = 通用
RemoveWindowFromScratchpad { #[proptest(strategy = "proptest::option::of(1..=5usize)")] id: Option<usize> },
```

M2a 先以 `#[proptest(skip)]` 加入（确定性用例可用、不参与随机生成），M2b 去掉 skip。`Op::Communicate` 需补隐藏队列分支（它直接遍历工作区）；激活用现有的 `Op::FocusWindow`（即 `activate_window`）。

`random_operations_dont_panic` 会把它们与增删输出、切工作区、手势、拖动、全屏、`AddWindowNextTo`、`SetParent` 等随机组合。这是发现审计遗漏最有效的手段，要多跑几轮（可临时调大 `cases`）。

**确定性用例**（新建 `src/layout/tests/scratchpad.rs`）：

| # | 场景 | 断言 |
|---|---|---|
| 1 | 浮动窗口隐藏 → 呈现 | 位置、尺寸与隐藏前一致 |
| 2 | 整列高的平铺窗口首次隐藏 → 呈现 | 请求尺寸 = 工作区 60%×60%（或规则值）；原列补位（开启功能一时视图被夹紧） |
| 3 | 三个窗口 A、B、C 依次隐藏，反复通用切换 | A→B→C→A |
| 4 | 带 id 呈现 | 不改变其他窗口在队列中的顺序 |
| 5 | A、B 已经由专属切换呈现，C 隐藏，聚焦普通窗口后按通用键 | 聚焦 seq 最大者（规则 2），C 不呈现 |
| 6 | 显示中切到平铺 / 全屏 / 最大化 / 移到其他工作区 | 状态 None |
| 7 | 显示中拖动并在同一工作区松手 | 仍为 Shown；拖到平铺 → None |
| 8 | 显示中切工作区（键盘、手势、`Monitor::add_tile` 激活） | 立即在新活动工作区（不依赖 refresh）；末尾空工作区不变量成立 |
| 9 | 跟随到底部空工作区 / 开 `empty-workspace-above-first` 跟随到首工作区 | 新增空工作区；active 下标正确；previous workspace 正确 |
| 10 | 未聚焦的暂存窗口跟随 | 目标工作区焦点不变 |
| 11 | 多个暂存窗口跟随 | 层叠顺序保持 |
| 12 | 隐藏中 `CloseWindow` | 队列为空，无 panic |
| 13 | 隐藏中移除全部输出再添加 | 队列完整；无输出时 `ToggleScratchpad`、`RemoveWindowFromScratchpad` 为无操作 |
| 14 | 显示中，所在显示器被移除 | 迁移到其他显示器的活动工作区 |
| 15 | 隐藏父窗口后 `AddWindowNextTo(parent)` | 父窗口被呈现，子窗口在其旁，无 panic；无输出时子窗口进入 Auto |
| 16 | 父窗口有可见子窗口时隐藏 | 无操作 |
| 17 | 隐藏中对其 id 执行 C/C' 类每个操作 | 无 panic、无状态变化 |
| 18 | 隐藏中提交新尺寸（`update_window`） | 无遗留动画 |
| 19 | 隐藏中 `activate_window` | 呈现并聚焦 |
| 20 | 同一窗口连续两次隐藏 | 第二次无操作，不重复入队 |
| 21 | 隐藏中 `remove-window-from-scratchpad` | 在当前工作区成为普通浮动窗口并聚焦 |

**客户端集成测试**（`src/tests/`，用现有 `Fixture`；可放到 M3 末尾）：隐藏后客户端继续提交与 ack configure；隐藏中取消映射；隐藏窗口创建带 parent 的 toplevel；IPC 事件流重建的状态等于完整查询（Shown→None 必须产生事件）。

**手动测试**：foot/kitty 下拉终端；Firefox（多弹窗、通知激活）；Spotify/Electron（自绘弹窗）；KeePassXC（对话框、托盘：确认 D11 的表现）；视频播放器隐藏后能正常息屏；DMS/bar 的窗口列表与任务栏点击；两块不同缩放的显示器之间呈现与插拔；MRU 打开时用 IPC 隐藏窗口。

---

## 5. 配置与 IPC（用户视角）

### 5.1 KDL

```kdl
layout {
    fill-view-after-removal
}

binds {
    // 切换类动作建议关闭按键重复，避免按住时快速闪烁
    Mod+Shift+Minus repeat=false { move-window-to-scratchpad; }
    Mod+Minus       repeat=false { scratchpad-show; }
    Mod+Grave       repeat=false { scratchpad-show app-id="^dropdown$"; }
    Mod+M           repeat=false { scratchpad-show app-id="^(Spotify|spotify)$"; }
}

window-rule {
    match app-id="^dropdown$"
    open-floating true
    default-column-width { proportion 0.8; }
    default-window-height { proportion 0.5; }
    default-floating-position x=0 y=0 relative-to="top"
}
```

### 5.2 `niri-config/src/binds.rs` 的 `Action`

```rust
MoveWindowToScratchpad,
#[knuffel(skip)]
MoveWindowToScratchpadById(u64),
ScratchpadShow(#[knuffel(property(name = "app-id"), str)] Option<RegexEq>),
#[knuffel(skip)]
ScratchpadShowById(u64),
RemoveWindowFromScratchpad,
#[knuffel(skip)]
RemoveWindowFromScratchpadById(u64),
```

`From<niri_ipc::Action>`：`ScratchpadShow { id: Some(id), .. }` → `ScratchpadShowById(id)`；`{ id: None, app_id }` → `ScratchpadShow(app_id.map(|s| RegexEq::from_str(&s)...))`。`validate_action` 已拒绝无效正则；万一转换失败（理论上不可达），用永不匹配的 `[^\s\S]` 并 `warn!`，不能退化成“匹配全部”，也不能 panic。其余两个照 `ToggleWindowFloating{,ById}` 的写法。注意 binds.rs 约 103 行注释：新动作也要加进 CLI 枚举。

### 5.3 动作分发（`src/input/mod.rs` `do_action()`，照抄 `ToggleWindowFloating{,ById}`，约 2156 行）

- `MoveWindowToScratchpad` / `ById`：`if let Some(w) = layout.move_to_scratchpad(..) { self.on_window_hidden_to_scratchpad(&w) }`。
- `ScratchpadShow(app_id)`：过滤器 `|m: &Mapped| app_id.as_ref().is_none_or(|re| with_toplevel_role(m.toplevel(), |r| r.app_id.as_deref().is_some_and(|a| re.0.is_match(a))))`；结果为 `Hidden(w)` 时执行副作用。
- `ScratchpadShowById(id)`：过滤器 `|m| m.id().get() == id`。
- `RemoveWindowFromScratchpad` / `ById`。
- 所有分支最后都调用 `queue_redraw_all()`。
- 光标（M3 实现时补充，照 `State::focus_window()`）：`scratchpad-show` 执行前记下活动显示器；结果为 `Focused` / `Shown` 时，活动显示器变了就 `maybe_warp_cursor_to_focus_centered()`，不成功再 `move_cursor_to_output()`；没变就 `maybe_warp_cursor_to_focus()`。`RemoveWindowFromScratchpadById` 作用于隐藏窗口（隐藏 → 呈现并聚焦）时同样处理；无 id 版本作用于聚焦窗口，不可能是隐藏窗口，不处理。`Hidden` / `Nothing` 与 `move-window-to-scratchpad` 不动光标（同 `close-window`）。

### 5.4 命令行

```bash
niri msg action move-window-to-scratchpad --id 42
niri msg action scratchpad-show --app-id '^dropdown$'
niri msg --json windows | jq '.[] | select(.is_in_scratchpad)'
```

下拉终端脚本须区分三种状态（否则窗口存在但未入暂存区时，`scratchpad-show` 没有反应）：

```bash
#!/usr/bin/env bash
w=$(niri msg --json windows | jq -c '[.[] | select(.app_id == "dropdown")][0] // empty')
if [ -z "$w" ]; then
    niri msg action spawn -- foot --app-id dropdown
elif [ "$(jq -r .is_in_scratchpad <<<"$w")" = true ]; then
    niri msg action scratchpad-show --id "$(jq -r .id <<<"$w")"
else
    niri msg action move-window-to-scratchpad --id "$(jq -r .id <<<"$w")"
fi
```

这个脚本有并发启动竞争（快速连按可能启动两个），不作为核心功能宣传。

文档：`docs/wiki/IPC.md`、`Configuration:-Key-Bindings.md` 中补充动作与字段说明，包括 §4.3 的“性质”和 §4.7 的空工作区行为。

---

## 6. 里程碑与提交

每个里程碑完成时：`cargo test` 全绿、`cargo clippy --all-targets` 无新警告、`cargo +nightly fmt --all`。每步一个或几个独立 commit，便于以后 rebase。

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | 基线：`cargo build`、`cargo test` 全绿（已完成编译缓存） | 记录基线测试数 |
| M1a ✅ | F1-1 配置 → F1-2 纯重构（单独提交，快照零变化）→ F1-3 → F1-4 → §3.6 用例 1–16（用例 10 仅拖动中部分：拖动中源工作区视图不变；落点后夹紧依赖 F1-5，归 M1b）、18 | 换成日常使用版本 |
| M1b ✅ | F1-5 拖走、F1-6 合并 + 用例 10、17 | 与 M1a 一起换成日常使用版本 |
| M2a | 暂存区 Layout 核心（上）：两步纯重构（§4.5.1 第 11 步清理辅助函数、§4.5.2 第 7 步 `ensure_empty_workspaces_after_add`）→ §4.5.0–§4.5.6（含 §4.3 五条规则、§4.5.1 第 4 步子窗口检查）→ 不变量 1–3、5–7 → §4.12 确定性用例 1–7、12、13、18–21。测试用的暂存区 Op 标 `#[proptest(skip)]`，不进随机生成 | 两步重构快照零变化；新用例全过并做反向验证；现有测试与快照不变 |
| M2b | 暂存区 Layout 核心（下）：§4.5.7 审计、§4.6 子窗口、§4.12 proptest Op（去掉 skip，进入随机生成）、用例 15–17 | `random_operations_dont_panic` 多轮无 panic。M2（a/b）中显示中的窗口不跟随，停留在呈现时的工作区；**为保证不变量 4 不被违反，M2 暂不启用不变量 4** |
| M3 | niri 集成：config 动作、IPC 动作/字段/变化检测/校验、`do_action`、§4.8 副作用与可见性（含 §4.8.1 D14/D15）、文档、客户端集成测试 | 可日常使用 |
| M4 | 跟随：§4.7 全部调用点 + 不变量 4 + 用例 8–11、14 | |
| M5 | 视觉：钉住渲染、专门的收起/展开动画（§8） | |
| M6 | 窗口规则 `open-in-scratchpad` / `is-in-scratchpad`；再映射时保留身份（可选） | |

工作量估算（含测试，业余时间）：M1 约 2–4 天；M2 约 5–8 天；M3 约 3–4 天；M4 约 2–3 天。原设计“150–250 行 / 3–4 周”只对功能一大致成立。

日常使用：用 AUR `niri-git` 的 PKGBUILD 打包，补丁放进 `prepare()` 中 `git am`；保留官方包在 `/var/cache/pacman/pkg/` 以便 `pacman -U` 回滚。嵌套测试：`cargo run -- -c /path/to/test-config.kdl`，内层用 Alt 作 Mod 键，避免与外层冲突。

### 6.1 进度

| 里程碑 | 状态 | 提交 | 测试 |
|---|---|---|---|
| M0 | ✅ 2026-10-05（工具链换成 rustup，rustc 1.99.0） | — | `cargo test --all` 245 / 0 失败（niri lib 222）；`RUN_SLOW_TESTS=1 cargo test --lib layout::` 130 |
| M1a | ✅ 2026-10-05 | `87949e15` 配置、`a541a30f` 提取吸附端点、`e303873d` 夹紧函数、`9833ba51` 调用点、`18fca982` 测试 | — |
| M1b | ✅ 2026-10-05 | `2fc8c299` F1-5、`1f38e79c` F1-6、`cabf5b38` 测试 | `cargo test --all` 249 / 0 失败（niri lib 226）；慢测试 134；`fill_view.rs` 共 21 个用例，新增触发点均做过反向验证 |
| 功能一手动矩阵（§3.6） | ⏳ 待做 | — | 日常使用版本已打包（M1a + M1b） |
| M2a | ✅ 2026-10-05 | `acc33e09` 清理辅助函数、`f6bb247f` `ensure_empty_workspaces_after_add`、`97c29645` 数据结构、`ef559bcf` 隐藏、`cf6206d7` 呈现与接口、`39bca733` 退出暂存区落点、`886a9374` 不变量、`70d20ac8` 测试 | `cargo test --all` 263 / 0 失败（niri lib 240）；慢测试 148；`scratchpad.rs` 共 14 个用例，关键实现点均做过反向验证 |
| M2b | ✅ 2026-10-05 | `69521a77` A 类、`4246ec5b` C/C'/C'' 类、`cda56e5b` 子窗口、`589b3c1d` proptest Op、`075ed4c9` 测试 | `cargo test --all` 267 / 0 失败（niri lib 244）；慢测试 152；`scratchpad.rs` 共 18 个用例；`random_operations_dont_panic` 10 轮 × 5000 cases，另有 256 + 3000 + 10000 cases 的单独运行，共约 6.3 万 cases，无 panic |

M1b 补充说明：

- 用例 10 的“落到别的工作区”用另一块显示器实现；Overview 中跨工作区放置走同一段按 id 查找源工作区的代码，未单独写用例。
- “拖回同一工作区只滑一次”：放置与夹紧在同一次调用内完成，单元测试中时间不推进，两段式本身不可观测；用例断言的是松手后动画终点即最终值、放回原位时视图完全不动。最终以手动测试为准。
- `source_workspace` 按本文用 `Option<WorkspaceId>`，实际总为 `Some`，可简化为 `WorkspaceId`（未改，待定）。
- 收尾 grep（§3.4）结果：所有移除路径均已触发或属于表格中的“否”；`swap_window_in_direction` 不改变列数，无需触发。

M2a 补充说明：

- `LayoutElement` 还有第三个实现：`niri-visual-tests/src/test_window.rs`。`cargo clippy --all-targets` 只检查根包，发现不了它；以后每步都用 `cargo clippy --workspace --all-targets`。数据结构提交当时漏改了它，已用 fixup 并入该提交，并逐个提交确认工作区编译、clippy 无警告。
- 数据结构单独提交时，新字段和 `set_tile_scratchpad` 尚未被使用，会产生 dead_code 警告。该提交临时加了 `#[allow(dead_code)]`，在后续“隐藏”“呈现”提交中随使用逐个去掉，最终代码中没有残留。
- proptest-derive 不允许在 `#[proptest(skip)]` 变体的字段上写 `strategy`（错误 E0028）。所以三个暂存区 Op 目前不带 strategy；M2b 去掉 skip 时要补上 §4.12 中的 `proptest::option::of(1..=5usize)`。
- 用例对照：1 `floating_window_keeps_geometry`；2 `tiled_window_first_shown_with_default_size`（含规则值、夹紧 off/on）；3 `generic_toggle_cycles_through_queue`；4 `toggle_by_id_keeps_queue_order`；5 `generic_toggle_focuses_most_recently_shown`；6 `leaving_floating_takes_out_of_scratchpad`；7 `interactive_move_of_shown_window`（含落到另一显示器活动工作区）；12 `close_hidden_window`；13 `hidden_windows_survive_output_removal`；18 `hidden_window_commit_does_not_animate`；19 `activating_hidden_window_shows_it`；20 `hiding_twice_is_noop`；21 `remove_hidden_window_from_scratchpad`。另加 `shown_window_elsewhere_is_brought_over`，覆盖规则 3（其他显示器、同显示器非活动工作区、源工作区变空后被清理）。
- 反向验证（逐个去掉后确认对应用例失败，再从 git 还原）：`Workspace::add_tile` 入口清标记、`toggle_window_floating` 浮动→平铺清标记、隐藏时的 `fill_view_after_removal()`、FIFO 的 `push_back`（改成 `push_front`）、D8 拖动落点恢复 Shown、`update_window` 隐藏分支的 `stop_all_animations()`。
- 未被用例覆盖：规则 3 搬动时递增 seq。M2a 中可以观察到，但需要多显示器加多个选择器的组合，未单独写用例。
- 规则 3 搬动窗口时没有开窗动画（文档未要求），窗口直接出现在当前工作区。是否加动画留到 M5 一并考虑。
- M3 注意：`scratchpad_state()` 是 `LayoutElement` 的 trait 方法，niri 层（IPC）调用时需要 `use crate::layout::LayoutElement`。

M2b 补充说明：

- A 类：隐藏 Tile 排在拖动中窗口和各工作区之后。M2a 的测试辅助函数 `state()` 去掉了多余的 `.chain(scratchpad_hidden_windows())`；`generic_toggle_cycles_through_queue` 原本把 `windows()` 当作“已显示的窗口”，改为按 Shown 状态过滤（`windows()` 含义变化导致，期望值不变）。
- 内部调用者的行为变化：`set_fullscreen()` 的 windowed fullscreen 分支和 `toggle_windowed_fullscreen()` 都经过 `with_windows_mut()`，A 类改动后会改到隐藏窗口，所以这两个方法的隐藏检查放在**函数最开头**（其余方法放在 interactive_move 检查之后）。`toggle_windowed_fullscreen()` 原本对隐藏 id 会在 `windows().find(..).unwrap()` 处 panic。
- `verify_invariants()` 的 Starting 分支在 `has_window()` 之外加了“不是隐藏窗口”的断言（`has_window()` 现在包含隐藏窗口，原断言变弱；这也是不变量 5 的延伸）。
- C 类中已自然满足、未加冗余检查（沿用 §4.5.4 的做法）：`scroll_amount_to_activate()`（遍历工作区，找不到返回 0）、`interactive_move_begin()`、`interactive_resize_begin()`、`interactive_resize_end()`。用例 17 逐个验证。
- §4.5.7 表格未列出、带窗口 id 的方法：`interactive_move_update()`、`interactive_move_end()`（只在 Starting/Moving 状态 id 匹配时动作，而 Starting/Moving 不可能是隐藏窗口）、`interactive_resize_update()`（遍历工作区，返回 false）、`start_open_animation_for_window()`（遍历工作区，无操作）。均自然满足，未加代码；经用户确认已补进 §4.5.7 表格。
- §4.6 第 3 条：`parent_changed` 经 `find_window_and_output_mut()` 找到隐藏父窗口后调用 `descendants_added()`，后者只遍历工作区，返回 false，无需代码。`bring_up_descendants_of()` 是有界循环，不受父链环影响。
- 测试框架：`AddWindowNextTo`（查重与查找 next_to）、`AddWindowToNamedWorkspace`（查重）、`SetParent` 补了隐藏队列分支；`AddWindow`、`SetForcedSize` 经 `has_window()` / `windows()` 自然覆盖。此前 `AddWindow` 等可能添加与隐藏窗口重复的 id，只是暂存区 Op 未参与随机生成所以没暴露。exhaustive 的 `operations_dont_panic` 列表未加入暂存区 Op（文档未要求）。
- 用例对照：15 `new_dialog_of_hidden_window_shows_it`（浮动/平铺对话框、`ActivateWindow::No`、无输出）；16 `hiding_with_visible_child_is_noop`（含规则 1 被拒返回 `Nothing`）；17 `operations_on_hidden_window_are_ignored`（隐藏窗口同时处于 windowed fullscreen，覆盖 `set_fullscreen(false)` 的特殊分支）。另加 `hidden_windows_are_found` 覆盖 A 类。`find_window_and_output(_mut)()` 无法用 `TestWindow` 测试（`is_wl_surface()` 恒为 false），留给 M3 的客户端集成测试。`SetParent` 作用于隐藏窗口只由随机测试覆盖。
- 反向验证（逐个去掉后确认对应用例失败，再从 git 还原）：`windows()`、`with_windows()`、`with_windows_mut()` 中的隐藏 Tile；`toggle_window_height`、`move_to_output`、`popup_target_rect`、`set_fullscreen`、`toggle_windowed_fullscreen` 的检查；`add_window` 中呈现隐藏父窗口、无输出改为 Auto；测试框架 `AddWindowNextTo` 的隐藏查找。
- 随机测试命中率（临时插桩，10000 cases）：成功隐藏约 200 次；各 C/C' 分支命中 1–8 次；NextTo 隐藏父窗口 4 次、无输出回退 1 次；`set_maximized`、`move_to_output` 的隐藏分支在该样本中未命中（用例 17 覆盖）。命中率低是因为 id 需要恰好对应已存在的窗口。未发现 panic。
- M3 注意（niri 层因 A 类改动产生的行为变化，目前没有隐藏窗口所以不可观测）：`focus-window-previous` 按焦点时间戳从 `windows()` 中选取，会选中刚隐藏的窗口并经 `activate_window` 把它呈现出来；`set-dynamic-cast-window --id` 可以指向隐藏窗口；`screenshot-window --id` 需要 monitor，对隐藏窗口无操作；`close-window --id` 有效（符合 §4.2）。用户已确认：前两者在 niri 层排除隐藏窗口（D14、D15，§4.8.1），M3 实现。另发现 portal 窗口选择列表也会列出隐藏窗口，记为 §4.8.1 待定项。

---

## 7. 实现陷阱清单（写代码时逐条对照）

1. `f64::clamp(min, max)` 在 `min > max` 时 panic：只在 `left < right` 时调用。
2. `interactive_move_end()` 先结束手势、后放回窗口：拖动相关的夹紧必须放在放置之后（F1-5）。
3. `Workspace::remove_tile()` 会 `output_leave`，`add_tile()` 会 `output_enter`；跟随（同一输出）不能走这两个函数，否则客户端会收到多余的 leave/enter。
4. `FloatingSpace::add_tile()` 插入在栈顶（下标 0）：批量加入时要从下往上加。
5. `floating_window_size` 是逻辑像素，`floating_pos` 是工作区比例。
6. `Monitor::add_tile()` 末尾的空工作区维护要提取成辅助函数复用，不能复制粘贴后漏掉 `empty_workspace_above_first`。
7. `add_workspace_at()` 会修正 `active_workspace_idx` 和切换动画的下标，跟随后不要自己再改。
8. 所有对父子关系的遍历只用 `is_child_of` 的直接关系，测试里可能构造出父链环。
9. 预提交钩子、提交处理必须能找到隐藏窗口（A 类），否则会打印 “pre-commit hook for mapped surfaces must be removed upon unmapping”，并跳过 dmabuf/事务处理。
10. IPC 的变化检测是逐字段比较的，新增字段必须加进 `changed` 判断。
11. 清可见性缓存要覆盖窗口的**全部**表面（`with_surfaces`，含子表面和弹出层），且对每个输出都要调用。
12. Tile 标记与 Mapped 状态只经 `set_tile_scratchpad()` 修改。
13. `remove_window_impl` 中隐藏队列分支返回的 `RemovedTile` 不能触发功能一，也不能进入空工作区清理逻辑。
14. 隐藏时 `set_activated(false)` 只是立即生效的提示，真正的 configure 由下一次 `Layout::refresh()` 的隐藏块发送。
15. niri-config 的解析快照、`niri-ipc` 的 JSON schema（若启用 `json-schema` feature）需随新字段更新。

---

## 8. 后续阶段要求（M5/M6，本轮不实现，仅作约束）

**钉住渲染（R10）**：为显示中的暂存窗口提供统一的“有效屏幕几何 + 层叠顺序”，让渲染、`window_under()`、`resize_edges_under()`、拖动起点（`mod.rs` 约 3950 行）、弹出菜单定位共享同一结果。定义它与目标工作区全屏窗口、bar、overlay 的层级。Overview 的进入/退出动画中不钉住。源、目标工作区的 layout 覆盖不同（strut/边框）时，以屏幕位置优先，松手后修正比例位置。

**专门动画（R16）**：快照按 window id + 转换代次管理，再次呈现时按 id 找到并丢弃旧快照，同一帧最多一份有效呈现。原工作区被清理、窗口退出、输出拔除时都要释放快照。反向打断时按当前呈现的几何/alpha 重新起算，不假定两条曲线可逆。custom-shader 不可逆时降级为直接完成。按 `--id` 隐藏非活动工作区上的窗口时不播放动画。整屏录屏仍能看到这段动画，可以接受。

**窗口规则**：`Match.is_in_scratchpad: Option<bool>`、`WindowRule.open_in_scratchpad: Option<bool>`。`open-in-scratchpad` 在首次映射后执行隐藏（不能在无尺寸时入队）。`is-in-scratchpad` 规则的重算不能反过来触发状态转换（防止进出循环）。

**再映射保留身份**：在 `Unmapped` 上记录 `was_in_scratchpad`；再映射时作为 Shown 呈现在当前工作区（不能直接隐藏，否则托盘“显示窗口”会没有反应）。

**拖动期间其他工作区的移除（M1b 发现）**：拖动中所有工作区的视图被 DnD 手势锁住，`fill_view_after_removal()` 会跳过；松手后只补夹紧源工作区。若需要，可在 `interactive_move_end()` 中对拖动期间发生过平铺移除的工作区（记录 id 集合）统一补一次夹紧。

**异步尺寸校正（R7）**：如有需要，加一次性的“移除后待校正”标记。下一次活动列提交尺寸后执行一次夹紧；任何手势、交互式 resize 或聚焦操作都会取消该标记。
