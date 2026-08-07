/**
 * 单元格层的**瓦片(tile)几何**——纯函数。
 *
 * # 为什么要瓦片
 *
 * 如果单元格画布正好等于可见区域,那么每滚动一像素都得往画布里画东西
 * (哪怕只补一条窄带)。而浏览器的合成器本来就能用 GPU **零成本地平移一个图层**。
 *
 * 于是把画布做得比可见区域大一圈(四周各留 [`TILE_MARGIN`] 的余量),滚动时:
 *
 * ```text
 * 瓦片仍然盖住可见区域  →  只改 CSS transform,主线程一个像素都不用画
 * 瓦片盖不住了(滚出余量)→  重新锚定瓦片并重绘(重叠部分靠位图平移复用)
 * ```
 *
 * 代价是画布内存变大(多出边距那一圈),换来的是绝大多数滚动帧的绘制开销归零。
 *
 * # 坐标约定
 *
 * 本模块内一律使用**设备像素**,与渲染器内部一致。
 */

import type { Scroll, Viewport } from "./geometry";

/**
 * 瓦片在可见区域之外每侧额外覆盖的边距(设备像素)。
 *
 * 太小则频繁重锚,失去意义;太大则每次重锚要画的面积变大、内存也涨。
 * 256 设备像素在常见行高下约等于 10 行,足以覆盖连续滚动中的绝大多数帧。
 */
export const TILE_MARGIN = 256;

/** 一块瓦片:内容坐标系里的一个矩形。 */
export interface Tile {
  /** 瓦片左上角对应的内容坐标 X。 */
  originX: number;
  /** 瓦片左上角对应的内容坐标 Y。 */
  originY: number;
  /** 瓦片宽度。 */
  width: number;
  /** 瓦片高度。 */
  height: number;
}

/** 内容总尺寸。 */
export interface ContentSize {
  width: number;
  height: number;
}

/**
 * 计算瓦片尺寸。
 *
 * 内容比可见区域还小时没必要留边距 —— 那点内容一次就画完了。
 */
export function tileSizeFor(body: Viewport, content: ContentSize): Viewport {
  return {
    width: Math.max(1, Math.min(Math.ceil(body.width) + TILE_MARGIN * 2, Math.ceil(content.width))),
    height: Math.max(
      1,
      Math.min(Math.ceil(body.height) + TILE_MARGIN * 2, Math.ceil(content.height)),
    ),
  };
}

/**
 * 为当前滚动位置锚定瓦片:让可见区域尽量落在瓦片正中,并夹在内容范围内。
 *
 * 居中而不是对齐左上角,是为了让「往回滚」也能享受到余量。
 */
export function anchorTile(
  scroll: Scroll,
  body: Viewport,
  tileSize: Viewport,
  content: ContentSize,
): Tile {
  const centerX = scroll.x + body.width / 2 - tileSize.width / 2;
  const centerY = scroll.y + body.height / 2 - tileSize.height / 2;
  return {
    originX: Math.round(clamp(centerX, 0, Math.max(0, content.width - tileSize.width))),
    originY: Math.round(clamp(centerY, 0, Math.max(0, content.height - tileSize.height))),
    width: tileSize.width,
    height: tileSize.height,
  };
}

/**
 * 瓦片是否仍然完整盖住**需要显示的区域**。
 *
 * 盖得住 → 这一帧的单元格层不需要重绘,改个 transform 就行。
 *
 * 注意要和内容边界求交:内容比可见区域小时(比如只有三列的表格),
 * 可见区域右侧本来就是空白,瓦片没有义务盖住那片空白 ——
 * 否则会永远判定为「盖不住」,退化成每帧全量重绘。
 */
export function tileCovers(
  tile: Tile,
  scroll: Scroll,
  body: Viewport,
  content: ContentSize,
): boolean {
  const needRight = Math.min(scroll.x + body.width, content.width);
  const needBottom = Math.min(scroll.y + body.height, content.height);
  return (
    tile.originX <= scroll.x &&
    tile.originY <= scroll.y &&
    tile.originX + tile.width >= needRight &&
    tile.originY + tile.height >= needBottom
  );
}

/**
 * 瓦片相对可见区域左上角应该平移多少(设备像素,通常为负)。
 *
 * 这个值直接喂给 CSS `transform: translate()`,由合成器在 GPU 上完成。
 */
export function tileTranslation(tile: Tile, scroll: Scroll): { x: number; y: number } {
  return { x: tile.originX - scroll.x, y: tile.originY - scroll.y };
}

/** 瓦片尺寸是否需要重建(可见区域或内容尺寸变了)。 */
export function tileSizeChanged(tile: Tile | null, size: Viewport): boolean {
  return tile === null || tile.width !== size.width || tile.height !== size.height;
}

function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, value));
}
