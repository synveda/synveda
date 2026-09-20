// FND-7: use the same WASM instructions on every host, including PNG encoding.
import CanvasKitInit from "canvaskit-wasm/full";
import { Canvg } from "canvg";
import { DOMParser } from "@xmldom/xmldom";

const CanvasKit = await CanvasKitInit();

export async function rasterize(source, width, height) {
  const canvas = CanvasKit.MakeCanvas(width, height);
  try {
    const context = canvas.getContext("2d");
    // CanvasKit has no DOM canvas. Canvg needs its fixed viewport dimensions;
    // delegate with the original receiver so Skia retains resource ownership.
    const svgContext = new Proxy(
      { canvas: { width, height } },
      {
        get(target, key) {
          if (key === "canvas") return target.canvas;
          const value = context[key];
          return typeof value === "function" ? value.bind(context) : value;
        },
        set(_target, key, value) {
          context[key] = value;
          return true;
        },
      },
    );
    const renderer = Canvg.fromString(svgContext, source, { DOMParser });
    renderer.resize(width, height);
    await renderer.render({ ignoreDimensions: true });
    return Buffer.from(canvas.toDataURL("image/png").split(",")[1], "base64");
  } finally {
    canvas.dispose();
  }
}
