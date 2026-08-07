# Spec-0001: Office 文件格式识别

- **状态**:已实现
- **关联**:RFC-0001、Story-0001
- **对应测试**:`crates/core/src/format.rs`(`#[cfg(test)] mod tests`)、`crates/core/src/lib.rs`

## 概述

从文件字节判断其为 Word(.docx)、Excel(.xlsx)、PowerPoint(.pptx)还是未知格式。
docx/xlsx/pptx 均为 OOXML,即 ZIP 容器,魔数 `PK\x03\x04`,通过容器内特征目录区分。

## 接口

```rust
pub enum Format { Docx, Xlsx, Pptx, Unknown }

/// 识别 office 格式;非 ZIP 或无法识别返回 Unknown。
pub fn detect_format(bytes: &[u8]) -> Format;
```

## 行为规格

| 输入 | 期望输出 | 说明 |
| --- | --- | --- |
| 空字节 `[]` | `Unknown` | 无魔数 |
| `"hello world"` | `Unknown` | 非 ZIP |
| ZIP 且含 `word/` | `Docx` | Word 文档 |
| ZIP 且含 `xl/` | `Xlsx` | Excel 表格 |
| ZIP 且含 `ppt/` | `Pptx` | PowerPoint 演示 |
| ZIP 但无已知特征目录 | `Unknown` | 其它 ZIP |

## 边界与错误

- 不做完整 ZIP 解析,仅扫描原始字节中的特征目录名(ZIP 本地文件头以未压缩形式存路径)。
- 长度小于魔数长度的输入直接判为 `Unknown`。

## 验收标准

- [x] 上表所有用例均通过单元测试。
- [x] `detect_format` 不 panic,任意字节输入都有确定返回。
- [x] `Format` 可 serde 序列化(供 WASM 传给前端)。
