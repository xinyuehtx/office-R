import { describe, it, expect } from "vitest";
import {
  anchorTile,
  tileCovers,
  tileSizeChanged,
  tileSizeFor,
  tileTranslation,
  TILE_MARGIN,
} from "./tile";

const BODY = { width: 800, height: 600 };
const BIG_CONTENT = { width: 100_000, height: 5_000_000 };

describe("tileSizeFor", () => {
  it("内容很大时,瓦片 = 可见区域 + 四周边距", () => {
    expect(tileSizeFor(BODY, BIG_CONTENT)).toEqual({
      width: BODY.width + TILE_MARGIN * 2,
      height: BODY.height + TILE_MARGIN * 2,
    });
  });

  it("内容比可见区域小时不留边距,避免白占内存", () => {
    expect(tileSizeFor(BODY, { width: 200, height: 100 })).toEqual({ width: 200, height: 100 });
  });

  it("内容只在一个方向上超出时,另一方向仍按内容裁剪", () => {
    const size = tileSizeFor(BODY, { width: 300, height: 5_000_000 });
    expect(size.width).toBe(300);
    expect(size.height).toBe(BODY.height + TILE_MARGIN * 2);
  });

  it("零内容不会产生非法尺寸", () => {
    const size = tileSizeFor(BODY, { width: 0, height: 0 });
    expect(size.width).toBeGreaterThan(0);
    expect(size.height).toBeGreaterThan(0);
  });
});

describe("anchorTile", () => {
  const tileSize = tileSizeFor(BODY, BIG_CONTENT);

  it("把可见区域摆在瓦片正中,两侧余量相等", () => {
    const tile = anchorTile({ x: 10_000, y: 20_000 }, BODY, tileSize, BIG_CONTENT);
    expect(10_000 - tile.originX).toBe(TILE_MARGIN);
    expect(20_000 - tile.originY).toBe(TILE_MARGIN);
  });

  it("滚到最顶端时瓦片贴住 0,不会出现负原点", () => {
    const tile = anchorTile({ x: 0, y: 0 }, BODY, tileSize, BIG_CONTENT);
    expect(tile.originX).toBe(0);
    expect(tile.originY).toBe(0);
  });

  it("滚到最末端时瓦片贴住内容边界,不会越界", () => {
    const scroll = { x: BIG_CONTENT.width - BODY.width, y: BIG_CONTENT.height - BODY.height };
    const tile = anchorTile(scroll, BODY, tileSize, BIG_CONTENT);
    expect(tile.originX + tile.width).toBe(BIG_CONTENT.width);
    expect(tile.originY + tile.height).toBe(BIG_CONTENT.height);
  });

  it("原点取整到整设备像素(否则 transform 会重采样发虚)", () => {
    const tile = anchorTile({ x: 1234.6, y: 987.4 }, BODY, tileSize, BIG_CONTENT);
    expect(Number.isInteger(tile.originX)).toBe(true);
    expect(Number.isInteger(tile.originY)).toBe(true);
  });

  it("锚定结果总是盖住可见区域", () => {
    for (const scroll of [
      { x: 0, y: 0 },
      { x: 137, y: 999 },
      { x: 50_000, y: 2_500_000 },
      { x: BIG_CONTENT.width - BODY.width, y: BIG_CONTENT.height - BODY.height },
    ]) {
      const tile = anchorTile(scroll, BODY, tileSize, BIG_CONTENT);
      expect(tileCovers(tile, scroll, BODY, BIG_CONTENT)).toBe(true);
    }
  });
});

describe("tileCovers", () => {
  const tile = { originX: 1000, originY: 1000, width: 1312, height: 1112 };

  it("可见区域完全在瓦片内 → 覆盖", () => {
    expect(tileCovers(tile, { x: 1100, y: 1100 }, BODY, BIG_CONTENT)).toBe(true);
  });

  it("刚好贴边也算覆盖", () => {
    expect(tileCovers(tile, { x: 1000, y: 1000 }, BODY, BIG_CONTENT)).toBe(true);
    expect(
      tileCovers(tile, { x: 1000 + tile.width - BODY.width, y: 1000 + tile.height - BODY.height }, BODY, BIG_CONTENT),
    ).toBe(true);
  });

  it("向任一方向滚出余量 → 不覆盖,需要重锚", () => {
    expect(tileCovers(tile, { x: 999, y: 1100 }, BODY, BIG_CONTENT)).toBe(false);
    expect(tileCovers(tile, { x: 1100, y: 999 }, BODY, BIG_CONTENT)).toBe(false);
    expect(tileCovers(tile, { x: 1000 + tile.width - BODY.width + 1, y: 1100 }, BODY, BIG_CONTENT)).toBe(false);
    expect(tileCovers(tile, { x: 1100, y: 1000 + tile.height - BODY.height + 1 }, BODY, BIG_CONTENT)).toBe(false);
  });

  it("内容比可见区域小时,不要求瓦片盖住空白区(否则会退化成每帧全量重绘)", () => {
    // 只有三列的表格:内容宽 300,可见区域宽 800,右侧 500 是空白
    const small = { width: 300, height: 200 };
    const tile = { originX: 0, originY: 0, width: 300, height: 200 };
    expect(tileCovers(tile, { x: 0, y: 0 }, BODY, small)).toBe(true);
  });

  it("在余量内连续滚动都不需要重锚 —— 这正是 GPU 平移能生效的区间", () => {
    const scroll = { x: 1256, y: 1256 }; // 居中锚定后的位置
    let covered = 0;
    for (let i = 0; i < TILE_MARGIN; i += 1) {
      if (tileCovers(tile, { x: scroll.x, y: scroll.y + i }, BODY, BIG_CONTENT)) covered += 1;
    }
    // 往下滚满一个边距的距离,几乎每一帧都不用重绘
    expect(covered).toBeGreaterThan(TILE_MARGIN * 0.9);
  });
});

describe("tileTranslation", () => {
  it("平移量 = 瓦片原点 - 滚动量", () => {
    const tile = { originX: 1000, originY: 2000, width: 1312, height: 1112 };
    expect(tileTranslation(tile, { x: 1100, y: 2200 })).toEqual({ x: -100, y: -200 });
  });

  it("瓦片与视口对齐时平移量为 0", () => {
    const tile = { originX: 0, originY: 0, width: 1312, height: 1112 };
    expect(tileTranslation(tile, { x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
  });

  it("滚动量是整设备像素时平移量也是整数(保证不重采样)", () => {
    const tile = { originX: 512, originY: 768, width: 1312, height: 1112 };
    const t = tileTranslation(tile, { x: 600, y: 900 });
    expect(Number.isInteger(t.x)).toBe(true);
    expect(Number.isInteger(t.y)).toBe(true);
  });
});

describe("tileSizeChanged", () => {
  it("没有瓦片时视为需要重建", () => {
    expect(tileSizeChanged(null, { width: 100, height: 100 })).toBe(true);
  });

  it("尺寸一致时不需要重建", () => {
    const tile = { originX: 0, originY: 0, width: 100, height: 200 };
    expect(tileSizeChanged(tile, { width: 100, height: 200 })).toBe(false);
  });

  it("任一边变化都需要重建", () => {
    const tile = { originX: 0, originY: 0, width: 100, height: 200 };
    expect(tileSizeChanged(tile, { width: 101, height: 200 })).toBe(true);
    expect(tileSizeChanged(tile, { width: 100, height: 201 })).toBe(true);
  });
});
