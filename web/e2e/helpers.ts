import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import type { Page } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));

/** 夹具绝对路径。 */
export function fixture(name: string): string {
  return resolve(here, "fixtures", name);
}

/** 切到某个标签页(文档/表格/演示)。 */
export async function gotoTab(page: Page, label: "文档" | "表格" | "演示") {
  await page.goto("/");
  await page.getByRole("tab", { name: label }).click();
}

/** 通过隐藏的 file input 上传夹具。 */
export async function upload(page: Page, name: string) {
  await page.locator('input[data-testid="file-input"]').setInputFiles(fixture(name));
}

/**
 * 采样某 canvas 是否画了非空内容(alpha 非 0 的像素数),用于断言「确实渲染了」。
 * 在页面内用离屏 2D 上下文重绘一份采样。
 */
export async function canvasNonEmptyPixels(page: Page, selector: string): Promise<number> {
  return page.evaluate((sel) => {
    const c = document.querySelector(sel) as HTMLCanvasElement | null;
    if (!c || !c.width || !c.height) return 0;
    const ctx = c.getContext("2d");
    if (!ctx) return 0;
    const h = Math.min(400, c.height);
    const data = ctx.getImageData(0, 0, c.width, h).data;
    let n = 0;
    for (let i = 3; i < data.length; i += 4) if (data[i] !== 0) n += 1;
    return n;
  }, selector);
}
