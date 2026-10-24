import { describe, expect, it } from "vitest";
import { createLatestRequestGate } from "./latestRequest.js";

describe("最新请求闸门", () => {
  it("刚登记的请求就是最新的", () => {
    const gate = createLatestRequestGate();

    const id = gate.begin();

    expect(gate.isLatest(id)).toBe(true);
  });

  it("新请求登记后，旧请求不再是最新的", () => {
    const gate = createLatestRequestGate();

    const older = gate.begin();
    const newer = gate.begin();

    expect(gate.isLatest(older)).toBe(false);
    expect(gate.isLatest(newer)).toBe(true);
  });

  it("多次并发刷新只有最后一次的结果算数", () => {
    const gate = createLatestRequestGate();

    const ids = [gate.begin(), gate.begin(), gate.begin()];

    expect(ids.filter((id) => gate.isLatest(id))).toStrictEqual([ids[2]]);
  });
});
