// ESLint 扁平配置(ESM;`web/package.json` 是 type: module)。
//
// tsconfig 的 noUnusedLocals / noUnusedParameters 只能覆盖「没用到的东西」,
// 挡不住这个仓库最容易出错的一类问题:canvas + hooks 的依赖数组。
// SheetCanvas / renderer / 三个页面里满是 useEffect / useCallback / useRef,
// 依赖写错既不报错也不崩,只表现为「偶尔不重绘」或「每帧重订阅」。
// react-hooks/exhaustive-deps 正是为此。
//
// no-console 则把 AGENTS.md「禁止把裸 console.log 留在代码里」这条人肉规矩自动化,
// 唯一豁免是 logger.ts —— 它就是那层封装。
import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/pkg/**", // wasm-pack 生成物(web/src/wasm/pkg 与 packages/*/pkg)
      "**/playwright-report/**",
      "**/test-results/**",
      "**/coverage/**",
      "crates/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "no-console": "error",
      // 未使用的变量按 TS 版规则走,并放行下划线前缀(约定俗成的「刻意忽略」)
      "no-unused-vars": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],

      // ---- 以下三条是 eslint-plugin-react-hooks v7 新增的编译器系规则 ----
      //
      // 它们命中的是**真实的设计债**,不是误报:三个页面都在 render 期写
      // `xxxRef.current = value`(见 PptPage 的 stepRef / WordPage 的 zoomRef),
      // 以及在 effect 里同步 setState(SheetCanvas 的冻结/过滤 effect)。
      // 正确的修法是把 draw 的依赖改走 ref、拆开 effect 的连锁 —— 那是一次
      // 独立的重构,不该混在「接入 lint」这一步里,更不该让 CI 先红着。
      //
      // 所以先降为 warn:问题保持可见、增量可控,修完再逐条提回 error。
      "react-hooks/refs": "warn",
      "react-hooks/set-state-in-effect": "warn",
      "react-hooks/immutability": "warn",
    },
  },
  {
    // 测试里允许 any 与非空断言:构造替身时收益大于风险
    files: ["**/*.test.{ts,tsx}", "src/test/**", "e2e/**"],
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-non-null-assertion": "off",
    },
  },
);
