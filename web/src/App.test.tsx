import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App 顶部导航", () => {
  it("默认展示文档(Word)页面", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: /文档 · Word/ })).toBeInTheDocument();
  });

  it("点击标签可切换到表格与演示页面", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("tab", { name: "表格" }));
    expect(screen.getByRole("heading", { name: /表格 · Excel/ })).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "演示" }));
    expect(screen.getByRole("heading", { name: /演示 · PowerPoint/ })).toBeInTheDocument();
  });

  it("每个页面都提供上传入口", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(screen.getByTestId("file-input")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "表格" }));
    expect(screen.getByTestId("file-input")).toBeInTheDocument();
  });
});
