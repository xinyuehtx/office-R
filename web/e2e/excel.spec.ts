import { test, expect } from "@playwright/test";
import { gotoTab, upload } from "./helpers";

test.describe("Excel 只读:公式 / 过滤 / 冻结", () => {
  test("公式求值:D2 = B2*C2 = 14", async ({ page }) => {
    await gotoTab(page, "表格");
    await upload(page, "sample.csv");
    const canvas = page.getByTestId("sheet-canvas");
    await expect(canvas).toBeVisible();
    // 移到 D2:ArrowRight×3 + ArrowDown
    await canvas.click();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowDown");
    // 状态栏播报当前格计算值 14,公式栏显示原始公式
    await expect(page.getByRole("status")).toContainText("D2:14");
    await expect(page.getByTestId("formula-bar")).toContainText("=B2*C2");
  });

  test("列过滤:单价 > 3 → 行数减少", async ({ page }) => {
    await gotoTab(page, "表格");
    await upload(page, "sample.csv");
    await expect(page.getByTestId("sheet-canvas")).toBeVisible();
    // 选到 B 列
    await page.getByTestId("sheet-canvas").click();
    await page.keyboard.press("ArrowRight");
    await page.getByLabel("过滤类型").selectOption("number");
    await page.getByLabel("数值运算").selectOption("gt");
    await page.getByLabel("数值", { exact: true }).fill("3");
    await page.getByTestId("filter-apply").click();
    // 生效标签出现,状态栏行数下降(表头 + 单价>3 的行)
    await expect(page.getByTestId("filter-active")).toContainText("B");
    await expect(page.getByTestId("sheet-status")).toContainText("行");
  });

  test("冻结:冻结首行后状态可见", async ({ page }) => {
    await gotoTab(page, "表格");
    await upload(page, "sample.csv");
    await expect(page.getByTestId("sheet-canvas")).toBeVisible();
    await page.getByRole("button", { name: "冻结首行" }).click();
    await expect(page.getByTestId("freeze-state")).toContainText("1 行");
  });

  test("xlsx:多工作表 + 公式缓存值 + 切换标签", async ({ page }) => {
    await gotoTab(page, "表格");
    await upload(page, "sample.xlsx");
    const canvas = page.getByTestId("sheet-canvas");
    await expect(canvas).toBeVisible();
    // 两张工作表的标签
    await expect(page.getByTestId("sheet-tabs")).toBeVisible();
    await expect(page.getByTestId("sheet-tab-0")).toContainText("数据");
    await expect(page.getByTestId("sheet-tab-1")).toContainText("第二表");
    // D2 = B2*C2,缓存值 14;公式栏回显 =B2*C2
    await canvas.click();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("status")).toContainText("D2:14");
    await expect(page.getByTestId("formula-bar")).toContainText("=B2*C2");
    // 切到第二张表
    await page.getByTestId("sheet-tab-1").click();
    await expect(canvas).toBeVisible();
  });
});
