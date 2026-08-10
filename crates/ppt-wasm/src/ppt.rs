//! PowerPoint (.pptx) 的 WASM 绑定。
//!
//! 解析(重 CPU)在 `office_core::pptx`;这里做跨边界搬运:
//! - `model` 把演示模型 serde 序列化(尺寸 + 幻灯 + 形状);
//! - 图片字节留在 WASM 内存,按下标取回;每张图带 `(幻灯序号, embed id)` 定位,
//!   因为不同幻灯的 rels 里 embed id 会重复。

use office_ppt::pptx::{self, ParsedPpt};
use wasm_bindgen::prelude::*;

/// 主线程持有的演示文稿句柄。
#[wasm_bindgen]
pub struct WasmPresentation {
    parsed: ParsedPpt,
    /// 与 `parsed.images` 同序的 (slide_index, embed) 列表,便于按下标查定位。
    locs: Vec<(usize, String)>,
}

#[wasm_bindgen]
impl WasmPresentation {
    /// 解析 pptx 字节。
    pub fn parse(bytes: &[u8]) -> Result<WasmPresentation, JsValue> {
        let parsed = pptx::parse(bytes).map_err(|e| JsValue::from_str(&e))?;
        // 反查 image_index 得到每张图的 (slide, embed)
        let mut locs = vec![(0usize, String::new()); parsed.images.len()];
        for ((slide, embed), &i) in &parsed.image_index {
            if i < locs.len() {
                locs[i] = (*slide, embed.clone());
            }
        }
        Ok(WasmPresentation { parsed, locs })
    }

    /// 演示模型(尺寸 + 幻灯 + 形状),serde 序列化为 JS 对象。
    #[wasm_bindgen(getter)]
    pub fn model(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.parsed.presentation)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// 图片数量。
    #[wasm_bindgen(getter, js_name = imageCount)]
    pub fn image_count(&self) -> usize {
        self.parsed.images.len()
    }

    /// 第 `i` 张图片所属幻灯序号(0 起)。
    #[wasm_bindgen(js_name = imageSlide)]
    pub fn image_slide(&self, i: usize) -> usize {
        self.locs.get(i).map(|(s, _)| *s).unwrap_or(0)
    }

    /// 第 `i` 张图片的 embed id(在其幻灯内唯一)。
    #[wasm_bindgen(js_name = imageEmbed)]
    pub fn image_embed(&self, i: usize) -> Option<String> {
        self.locs.get(i).map(|(_, e)| e.clone())
    }

    /// 第 `i` 张图片的 MIME。
    #[wasm_bindgen(js_name = imageMime)]
    pub fn image_mime(&self, i: usize) -> Option<String> {
        self.parsed.images.get(i).map(|img| img.mime.clone())
    }

    /// 第 `i` 张图片的字节。
    #[wasm_bindgen(js_name = imageBytes)]
    pub fn image_bytes(&self, i: usize) -> Vec<u8> {
        self.parsed
            .images
            .get(i)
            .map(|img| img.data.clone())
            .unwrap_or_default()
    }
}
