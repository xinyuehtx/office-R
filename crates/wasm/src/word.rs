//! Word (.docx) 的 WASM 绑定。
//!
//! 解析(重 CPU)在 `office_core::docx`;这里只做跨边界搬运:
//! - `model` 把文档模型 serde 序列化为 JS 对象(段落/表格/内联);
//! - 图片字节留在 WASM 线性内存,按 `id` 用 `imageBytes` 取回(避免 base64 膨胀),
//!   JS 侧用 Blob + object URL 交给 canvas `drawImage`。

use office_core::docx::{self, ParsedDoc};
use wasm_bindgen::prelude::*;

/// 主线程持有的 Word 文档句柄。
#[wasm_bindgen]
pub struct WasmWordDoc {
    parsed: ParsedDoc,
}

#[wasm_bindgen]
impl WasmWordDoc {
    /// 解析 docx 字节。失败返回 JS 错误。
    pub fn parse(bytes: &[u8]) -> Result<WasmWordDoc, JsValue> {
        docx::parse(bytes)
            .map(|parsed| WasmWordDoc { parsed })
            .map_err(|e| JsValue::from_str(&e))
    }

    /// 文档模型(段落/表格/内联),serde 序列化为 JS 对象。
    #[wasm_bindgen(getter)]
    pub fn model(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.parsed.doc)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// 图片数量。
    #[wasm_bindgen(getter, js_name = imageCount)]
    pub fn image_count(&self) -> usize {
        self.parsed.images.len()
    }

    /// 第 `i` 张图片的关系 id(与内联 `Image.id` 对应)。
    #[wasm_bindgen(js_name = imageId)]
    pub fn image_id(&self, i: usize) -> Option<String> {
        self.parsed.images.get(i).map(|img| img.id.clone())
    }

    /// 第 `i` 张图片的 MIME。
    #[wasm_bindgen(js_name = imageMime)]
    pub fn image_mime(&self, i: usize) -> Option<String> {
        self.parsed.images.get(i).map(|img| img.mime.clone())
    }

    /// 第 `i` 张图片的字节(拷贝到 JS 的 Uint8Array)。
    #[wasm_bindgen(js_name = imageBytes)]
    pub fn image_bytes(&self, i: usize) -> Vec<u8> {
        self.parsed
            .images
            .get(i)
            .map(|img| img.data.clone())
            .unwrap_or_default()
    }
}
