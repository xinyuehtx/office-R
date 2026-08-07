/**
 * 共享文本测量缓存(参考 chenglou/pretext 的测量与缓存思路,自研实现)。
 *
 * 设计要点(来自 pretext 的 `measurement.ts`,取其精华、不引入依赖):
 * - **canvas `measureText`**:不碰 DOM、可在 Worker/OffscreenCanvas 下工作;
 * - **两级缓存** `font → (segment → width)`:字体串命名空间,段级缓存宽度。
 *   表格/文档里同一批 token 会被反复测量,段级缓存命中率极高;
 * - **字体加载失效**:`document.fonts.ready` 后清空缓存,避免用回退字体测出的旧宽度;
 * - 只做**单行宽度测量 + 省略号裁剪**(电子表格/文档单元格所需),不做多行折行 ——
 *   那是 pretext 的主场,但我们用不上,故不引入其 ~40KB 的 bidi/kinsoku。
 *
 * 三个页面(excel/word/ppt)共用一个实例,缓存跨页面复用。
 */

/** 惰性创建的测量上下文(优先 OffscreenCanvas,退回 DOM canvas)。 */
function createMeasureContext(): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null {
  try {
    if (typeof OffscreenCanvas !== "undefined") {
      const ctx = new OffscreenCanvas(1, 1).getContext("2d");
      if (ctx) return ctx;
    }
  } catch {
    // 某些环境 OffscreenCanvas 存在但 getContext 抛错,退回 DOM
  }
  if (typeof document !== "undefined") {
    return document.createElement("canvas").getContext("2d");
  }
  return null;
}

/**
 * 文本测量器:按字体分级缓存单段宽度,并提供裁剪到宽度(省略号)。
 *
 * 传入的 `font` 是完整的 CSS font 简写(如 `"bold 14px system-ui"`)。
 */
export class TextMeasurer {
  /** font → (segment → width)。 */
  private readonly caches = new Map<string, Map<string, number>>();
  private ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null | undefined;
  private currentFont = "";
  private fontsHooked = false;

  /** 测量单段文本在给定字体下的宽度(CSS 像素)。结果按 (font, text) 缓存。 */
  measure(text: string, font: string): number {
    if (text === "") return 0;
    this.hookFontLoad();
    let cache = this.caches.get(font);
    if (!cache) {
      cache = new Map();
      this.caches.set(font, cache);
    }
    const cached = cache.get(text);
    if (cached !== undefined) return cached;

    const ctx = this.context();
    if (!ctx) {
      // 无 canvas(如极简测试环境):用字符数 × 估算宽度兜底
      const est = text.length * 8;
      cache.set(text, est);
      return est;
    }
    if (this.currentFont !== font) {
      ctx.font = font;
      this.currentFont = font;
    }
    const width = ctx.measureText(text).width;
    cache.set(text, width);
    return width;
  }

  /**
   * 把文本裁剪到 `maxWidth`,超出则以省略号结尾。
   *
   * 用**二分**在字符边界上找最长可放下的前缀 —— O(log n) 次测量,
   * 而不是逐字符累加。返回可直接绘制的字符串。
   */
  fit(text: string, maxWidth: number, font: string): string {
    if (text === "" || maxWidth <= 0) return "";
    if (this.measure(text, font) <= maxWidth) return text;

    const ellipsis = "…";
    const ellipsisWidth = this.measure(ellipsis, font);
    if (ellipsisWidth > maxWidth) return "";

    const chars = Array.from(text); // 按码点切,避免切碎代理对
    let lo = 0;
    let hi = chars.length;
    // 找最大的 k 使得 前 k 个字符 + 省略号 宽度 <= maxWidth
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      const w = this.measure(chars.slice(0, mid).join(""), font) + ellipsisWidth;
      if (w <= maxWidth) lo = mid;
      else hi = mid - 1;
    }
    return chars.slice(0, lo).join("") + ellipsis;
  }

  /**
   * 把一串文本按最大宽度**折行**(用于 Word 的流式布局)。
   *
   * 优先在空白处断行(西文),放不下单个「词」时退到按字符断(CJK / 长串)。
   * 返回每行的文本;`maxWidth <= 0` 时整体作一行。
   */
  wrap(text: string, maxWidth: number, font: string): string[] {
    if (text === "") return [""];
    if (maxWidth <= 0) return [text];

    const lines: string[] = [];
    // 先按显式换行拆
    for (const rawLine of text.split("\n")) {
      if (rawLine === "") {
        lines.push("");
        continue;
      }
      let current = "";
      // 以「词 + 其后空白」为单位推进;CJK 无空白时逐字符
      const tokens = rawLine.match(/\s+|[^\s]+/g) ?? [rawLine];
      for (const token of tokens) {
        const candidate = current + token;
        if (this.measure(candidate, font) <= maxWidth) {
          current = candidate;
          continue;
        }
        // 放不下:先把已有内容收行
        if (current !== "") {
          lines.push(current);
          current = "";
        }
        // 单个 token 仍放不下 → 按字符硬断
        if (this.measure(token, font) > maxWidth) {
          let piece = "";
          for (const ch of token) {
            if (this.measure(piece + ch, font) <= maxWidth) {
              piece += ch;
            } else {
              if (piece !== "") lines.push(piece);
              piece = ch;
            }
          }
          current = piece;
        } else {
          current = token;
        }
      }
      lines.push(current);
    }
    // 行首前导空白在换行后无意义,去掉(西文换行习惯)
    return lines.map((l, i) => (i === 0 ? l : l.replace(/^\s+/, "")));
  }

  /** 清空所有缓存(字体变化 / 主动重置时用)。 */
  clear(): void {
    this.caches.clear();
    this.currentFont = "";
  }

  private context(): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null {
    if (this.ctx === undefined) this.ctx = createMeasureContext();
    return this.ctx;
  }

  /** Web 字体加载完成后测出的宽度会变,监听一次并清缓存。 */
  private hookFontLoad(): void {
    if (this.fontsHooked) return;
    this.fontsHooked = true;
    const fonts = (globalThis as { document?: { fonts?: { ready?: Promise<unknown> } } }).document
      ?.fonts;
    if (fonts?.ready && typeof fonts.ready.then === "function") {
      fonts.ready.then(() => this.clear()).catch(() => {});
    }
  }
}

/** 全局共享实例:三个页面共用,缓存跨页面复用。 */
export const sharedMeasurer = new TextMeasurer();
