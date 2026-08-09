/**
 * 共享的 canvas 图表绘制:柱 / 线 / 饼。
 *
 * Excel(内嵌图表)与 PPT(幻灯图表)共用同一套简洁绘制,保证视觉一致、不重复实现。
 * 只画数据本身 + 标题 + 白底边框;坐标轴刻度/图例/数据标签非目标。
 */

/** 可渲染的图表数据(与 Rust `chart::ChartData` 的 serde 输出对应)。 */
export interface ChartData {
  kind: string;
  series: number[][];
  categories: string[];
  title?: string | null;
}

/** 分类色板。 */
export const CHART_COLORS = ["#4c78a8", "#f58518", "#54a24b", "#e45756", "#72b7b2", "#eeca3b"];

/**
 * 在矩形 `(x,y,w,h)` 内绘制图表:白底 + 边框 + 可选标题 + 柱/线/饼。
 *
 * `fontSize` 为标题字号(设备/CSS 像素,由调用方按缩放传入)。
 */
export function drawChartInRect(
  ctx: CanvasRenderingContext2D,
  chart: ChartData,
  x: number,
  y: number,
  w: number,
  h: number,
  fontSize: number,
  fontFamily: string,
): void {
  ctx.save();
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(x, y, w, h);
  ctx.strokeStyle = "#c4ccd4";
  ctx.lineWidth = 1;
  ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);

  const pad = Math.max(6, fontSize * 0.5);
  let top = y + pad;
  if (chart.title) {
    ctx.fillStyle = "#1f2328";
    ctx.font = `${fontSize}px ${fontFamily}`;
    ctx.textBaseline = "top";
    ctx.textAlign = "center";
    ctx.fillText(chart.title, x + w / 2, top);
    top += fontSize * 1.4;
  }
  const plot = { x: x + pad, y: top, w: w - pad * 2, h: y + h - pad - top };
  if (plot.w <= 0 || plot.h <= 0 || chart.series.length === 0) {
    ctx.restore();
    return;
  }

  const flat = chart.series.flat();
  const maxV = Math.max(1e-9, ...flat.map((v) => Math.abs(v)));

  if (chart.kind === "pie") {
    const vals = chart.series[0] ?? [];
    const total = vals.reduce((a, b) => a + Math.abs(b), 0) || 1;
    const cx = plot.x + plot.w / 2;
    const cy = plot.y + plot.h / 2;
    const r = Math.min(plot.w, plot.h) / 2;
    let a0 = -Math.PI / 2;
    vals.forEach((v, i) => {
      const a1 = a0 + (Math.abs(v) / total) * Math.PI * 2;
      ctx.fillStyle = CHART_COLORS[i % CHART_COLORS.length];
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.arc(cx, cy, r, a0, a1);
      ctx.closePath();
      ctx.fill();
      a0 = a1;
    });
  } else if (chart.kind === "line") {
    chart.series.forEach((ser, si) => {
      ctx.strokeStyle = CHART_COLORS[si % CHART_COLORS.length];
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ser.forEach((v, i) => {
        const px = plot.x + (ser.length <= 1 ? 0 : (i / (ser.length - 1)) * plot.w);
        const py = plot.y + plot.h - (v / maxV) * plot.h;
        if (i === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      });
      ctx.stroke();
    });
  } else {
    // bar:各类别分组
    const nCat = Math.max(...chart.series.map((s) => s.length), 1);
    const nSer = chart.series.length;
    const groupW = plot.w / nCat;
    const barW = (groupW * 0.8) / nSer;
    for (let c = 0; c < nCat; c += 1) {
      for (let s = 0; s < nSer; s += 1) {
        const v = chart.series[s][c] ?? 0;
        const bh = (Math.abs(v) / maxV) * plot.h;
        const bx = plot.x + c * groupW + groupW * 0.1 + s * barW;
        ctx.fillStyle = CHART_COLORS[s % CHART_COLORS.length];
        ctx.fillRect(bx, plot.y + plot.h - bh, Math.max(1, barW - 1), bh);
      }
    }
  }
  ctx.restore();
}
