/**
 * canvas 测试替身。
 *
 * jsdom 没有实现 2D 上下文(`getContext("2d")` 返回 `null`),
 * 但渲染管线的正确性恰恰体现在「发出了哪些绘制指令」上,
 * 所以这里做一个**记录型**上下文:调用照单全收,测试再对记录下断言。
 *
 * 这也让 `save`/`restore` 是否配对、`clip` 是否设置这类容易出错的地方
 * 变成可断言的事实,而不是只能靠肉眼看截图。
 */

/** 一次绘制调用的记录。 */
export interface DrawCall {
  method: string;
  args: unknown[];
}

/** 记录型 2D 上下文。 */
export interface RecordingContext extends CanvasRenderingContext2D {
  /** 所有调用记录。 */
  readonly calls: DrawCall[];
  /** 清空记录。 */
  reset(): void;
  /** 统计某个方法被调用的次数。 */
  countOf(method: string): number;
  /** `save` 与 `restore` 的净差值;正确配对时为 0。 */
  saveDepth(): number;
  /** 记录中出现过的文本(用于断言画了哪些单元格)。 */
  texts(): string[];
}

/** 需要被记录的方法名。 */
const RECORDED_METHODS = [
  "save",
  "restore",
  "beginPath",
  "closePath",
  "clip",
  "rect",
  "roundRect",
  "moveTo",
  "lineTo",
  "arcTo",
  "fill",
  "stroke",
  "fillRect",
  "clearRect",
  "strokeRect",
  "fillText",
  "drawImage",
  "setTransform",
  "translate",
  "scale",
] as const;

/** 创建一个记录型上下文。`charWidth` 决定 `measureText` 的返回值。 */
export function createRecordingContext(charWidth = 7): RecordingContext {
  const calls: DrawCall[] = [];
  const target: Record<string, unknown> = {
    // 这些属性会被绘制代码赋值,保留最后一次的值即可
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    font: "",
    textAlign: "left",
    textBaseline: "alphabetic",
    measureText: (text: string) => ({ width: text.length * charWidth }),
    canvas: { width: 0, height: 0 },
  };

  for (const method of RECORDED_METHODS) {
    target[method] = (...args: unknown[]) => {
      calls.push({ method, args });
    };
  }

  target.calls = calls;
  target.reset = () => {
    calls.length = 0;
  };
  target.countOf = (method: string) => calls.filter((call) => call.method === method).length;
  target.saveDepth = () =>
    calls.reduce(
      (depth, call) =>
        call.method === "save" ? depth + 1 : call.method === "restore" ? depth - 1 : depth,
      0,
    );
  target.texts = () =>
    calls.filter((call) => call.method === "fillText").map((call) => String(call.args[0]));

  return target as unknown as RecordingContext;
}

/** 一个足以喂给渲染器的假 canvas 元素。 */
export function createStubCanvas(
  width = 800,
  height = 600,
  charWidth = 7,
): { canvas: HTMLCanvasElement; ctx: RecordingContext } {
  const ctx = createRecordingContext(charWidth);
  const canvas = {
    width,
    height,
    style: {} as CSSStyleDeclaration,
    dataset: {} as DOMStringMap,
    children: [] as unknown[],
    getContext: () => ctx,
    appendChild: (child: unknown) => {
      (canvas.children as unknown[]).push(child);
      return child;
    },
    remove: () => {},
  };
  (ctx as unknown as { canvas: unknown }).canvas = canvas;
  return { canvas: canvas as unknown as HTMLCanvasElement, ctx };
}

/** 渲染器建出来的图层结构(替身版)。 */
export interface StubLayers {
  /** 挂载容器。 */
  container: HTMLElement;
  /** 元素工厂,传给 `GridRenderer` 的 `createElement`。 */
  createElement: (tag: "canvas" | "div") => HTMLElement;
  /** 按 `data-layer` 取某一层的记录型上下文。 */
  layer(name: "body" | "headers" | "overlay"): RecordingContext | undefined;
  /** 按 `data-layer` 取某一层的画布。 */
  canvas(name: "body" | "headers" | "overlay"): HTMLCanvasElement | undefined;
  /** 已创建的画布总数(含位图平移用的暂存层)。 */
  readonly created: { canvas: HTMLCanvasElement; ctx: RecordingContext }[];
}

/**
 * 建一套图层替身:容器 + 元素工厂。
 *
 * 渲染器会自己 `createElement("div"/"canvas")` 并 `appendChild`,
 * 这里把这两步都换成可观测的假实现,于是「每层各画了什么」变成可断言的事实。
 */
export function createStubLayers(charWidth = 7): StubLayers {
  const created: { canvas: HTMLCanvasElement; ctx: RecordingContext }[] = [];

  const makeDiv = (): HTMLElement => {
    const div = {
      style: {} as CSSStyleDeclaration,
      children: [] as unknown[],
      appendChild: (child: unknown) => {
        (div.children as unknown[]).push(child);
        return child;
      },
      remove: () => {},
    };
    return div as unknown as HTMLElement;
  };

  const container = makeDiv();

  const createElement = (tag: "canvas" | "div"): HTMLElement => {
    if (tag === "div") return makeDiv();
    const stub = createStubCanvas(0, 0, charWidth);
    created.push(stub);
    return stub.canvas as unknown as HTMLElement;
  };

  const find = (name: string) =>
    created.find((entry) => (entry.canvas as unknown as { dataset: DOMStringMap }).dataset?.layer === name);

  return {
    container,
    createElement,
    created,
    layer: (name) => find(name)?.ctx,
    canvas: (name) => find(name)?.canvas,
  };
}
