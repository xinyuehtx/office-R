import { test, expect } from "@playwright/test";
import { gotoTab, upload, canvasNonEmptyPixels } from "./helpers";

test.describe("PPT 只读:渲染 / 导航 / 演示模式", () => {
  test("上传 pptx → 渲染当前幻灯,缩略图与翻页", async ({ page }) => {
    await gotoTab(page, "演示");
    await upload(page, "sample.pptx");
    await expect(page.getByTestId("ppt-stats")).toContainText("2 张幻灯片");
    await expect(page.getByTestId("ppt-canvas")).toBeVisible();
    await expect
      .poll(() => canvasNonEmptyPixels(page, 'canvas[data-testid="ppt-canvas"]'))
      .toBeGreaterThan(1000);

    // 翻到第二页
    await page.getByRole("button", { name: /下一张/ }).click();
    await expect(page.getByTestId("ppt-page")).toContainText("2 / 2");
    // 第二页含切换 + 动画标记(fixture 里带 transition/timing)
    await expect(page.getByTestId("ppt-badge-transition")).toBeVisible();
    await expect(page.getByTestId("ppt-badge-animation")).toBeVisible();
    // 缩放控件:放大后百分比变化
    await page.getByTestId("ppt-zoom-in").click();
    await expect(page.getByTestId("ppt-zoom-reset")).toContainText("125%");
  });

  test("演示模式:进入全屏,方向键翻页,Esc 退出", async ({ page }) => {
    await gotoTab(page, "演示");
    await upload(page, "sample.pptx");
    await expect(page.getByTestId("ppt-canvas")).toBeVisible();

    await page.getByTestId("ppt-present").click();
    // 全屏遮罩容器出现
    await expect(page.locator(".ppt-layout--present")).toBeVisible();
    // 方向键翻页(演示态复用同一 canvas)
    await page.keyboard.press("ArrowRight");
    await expect(page.getByTestId("ppt-canvas")).toBeVisible();
    // Esc 退出
    await page.keyboard.press("Escape");
    await expect(page.locator(".ppt-layout--present")).toHaveCount(0);
  });
});
