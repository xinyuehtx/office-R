# 开发工作流(SDD + TDD)

本项目采用**规格驱动开发(SDD)** 与 **测试驱动开发(TDD)** 结合的流程。
所有功能从需求出发,经过 RFC 评审、规格与测试先行,再进入实现。

## 总览

```
需求
 └─▶ ① RFC(docs/rfcs/)          方案设计,供评审
      └─▶ ② Spec + Story + Test  规格、用户故事、失败测试先行
           └─▶ ③ 实现(SDD+TDD)  写代码让测试变绿
                └─▶ ④ CI 绿灯     fmt/clippy/test + typecheck/test/build
                     └─▶ ⑤ 部署   GitHub Pages
```

## 各阶段产物

| 阶段 | 目录 | 说明 |
| --- | --- | --- |
| ① RFC | `docs/rfcs/NNNN-title.md` | 动机、方案、取舍、影响、未决问题。合并前需评审。 |
| ② Spec | `docs/specs/NNNN-title.md` | 精确的输入/输出/边界与**验收标准**,是测试与实现的依据。 |
| ② Story | `docs/stories/NNNN-title.md` | 用户故事 + Given/When/Then 验收场景。 |
| ② Test | `crates/*/src/*.rs`(`#[cfg(test)]`)、`web/src/**/*.test.tsx` | **先写失败测试**,对应 Spec 的验收标准。 |
| ③ 实现 | `crates/`、`web/` | 让测试从红变绿,只写让测试通过所需的代码。 |

## TDD 循环

1. **红**:依据 Spec 写测试,运行确认失败。
2. **绿**:写最小实现让测试通过。
3. **重构**:在测试保护下清理代码;保持 `cargo fmt`、`clippy`、`tsc` 无警告。

## 编号规则

RFC / Spec / Story 使用四位递增编号,同一功能三者尽量复用同一编号(如 `0001`)。

## 提交前检查清单(强制)

清单只有一份,在 [AGENTS.md 的「提交前检查清单」](../AGENTS.md#提交前检查清单强制)。

此前这里抄了一份平行清单,两边已经漂移(一处写 `pnpm -C web typecheck`、
一处写「`web` 下 `pnpm typecheck`」),而两份都自称「强制」—— 这正是
「AGENTS.md 是单一事实来源」要避免的情况。

## 分支与提交

- 从 `main` 切功能分支开发,PR 合并回 `main`。
- 合并到 `main` 触发 GitHub Pages 部署。
- 提交信息使用简洁的中文或英文祈使句,一次提交聚焦一件事。
