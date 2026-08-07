/**
 * 输入事件 → 视图动作的**纯函数**换算。
 *
 * 单独抽出来的理由:滚轮的语义比看上去复杂(三种 `deltaMode`、
 * 触控板与鼠标的差异、Shift 横滚、Ctrl 缩放),把它埋在事件回调里
 * 就只能靠手点来验证。放在这里可以直接单测。
 */

/** 滚动增量(CSS 像素)。 */
export interface ScrollDelta {
  dx: number;
  dy: number;
}

/** 换算滚轮事件所需的上下文。 */
export interface WheelContext {
  /** `deltaMode = LINE` 时一行等于多少像素。 */
  lineHeight: number;
  /** `deltaMode = PAGE` 时一页等于多少像素。 */
  pageHeight: number;
}

/** 只取用到的字段,便于测试构造。 */
export interface WheelLike {
  deltaX: number;
  deltaY: number;
  /** 0 = 像素,1 = 行,2 = 页。 */
  deltaMode: number;
  shiftKey: boolean;
}

/** `deltaMode` 到像素的换算系数。 */
function deltaScale(deltaMode: number, context: WheelContext): number {
  if (deltaMode === 1) return context.lineHeight;
  if (deltaMode === 2) return context.pageHeight;
  return 1;
}

/**
 * 把滚轮事件换算成滚动增量。
 *
 * **Shift + 滚轮 = 横向滚动**:这是鼠标用户唯一的横滚方式,浏览器并不会
 * 替我们把 `deltaY` 转成 `deltaX`(只有部分平台会),所以必须自己处理。
 * 已经给出 `deltaX` 的平台不再二次转换,否则会横滚两倍。
 *
 * 触控板本来就会同时给出 `deltaX`/`deltaY`,直接透传即可获得双向滚动。
 */
export function wheelToScrollDelta(event: WheelLike, context: WheelContext): ScrollDelta {
  const scale = deltaScale(event.deltaMode, context);
  if (event.shiftKey && event.deltaX === 0) {
    return { dx: event.deltaY * scale, dy: 0 };
  }
  return { dx: event.deltaX * scale, dy: event.deltaY * scale };
}

/** Ctrl/⌘ + 滚轮的缩放灵敏度。 */
const ZOOM_SENSITIVITY = 0.0015;

/**
 * 把滚轮事件换算成缩放倍率系数(相对当前缩放)。
 *
 * 用指数而不是线性:这样「放大再缩小同样的距离」能回到原点,
 * 手感上也更接近地图类应用。
 */
export function wheelToZoomFactor(event: WheelLike, context: WheelContext): number {
  const scale = deltaScale(event.deltaMode, context);
  return Math.exp(-event.deltaY * scale * ZOOM_SENSITIVITY);
}
