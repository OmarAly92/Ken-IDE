import { describe, expect, it } from "vitest";
import { APP_NAME, computeWindowTitle } from "./useWindowTitle";

describe("computeWindowTitle", () => {
  it("falls back to the app name when nothing else is available", () => {
    expect(computeWindowTitle("", "")).toBe("Ken IDE");
    expect(APP_NAME).toBe("Ken IDE");
  });

  it("shows the project alone when the tab label equals the project", () => {
    expect(computeWindowTitle("ken-ide", "ken-ide")).toBe("ken-ide");
  });

  it("shows the project alone when there is no tab label", () => {
    expect(computeWindowTitle("ken-ide", "")).toBe("ken-ide");
  });

  it("joins project and label when they differ", () => {
    expect(computeWindowTitle("ken-ide", "src")).toBe("ken-ide — src");
  });

  it("shows the label alone when there is no project", () => {
    expect(computeWindowTitle("", "Settings")).toBe("Settings");
  });
});
