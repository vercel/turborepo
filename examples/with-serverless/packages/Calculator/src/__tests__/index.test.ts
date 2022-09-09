import { Calculator } from "..";

describe("simple calculator", () => {
  let calc: Calculator;
  beforeAll(() => {
    calc = new Calculator({
      precision: 1
    });

  });

  it("test add operator", () => {
    let ans = calc.add({
      x: 1,
      y: 2
    });

    expect(ans).toBe("3");
  });

  it("test subtract operator", () => {
    let ans = calc.subtract({
      x: 3,
      y: 1
    });

    expect(ans).toBe("2");
  });

  it("test divide operator", () => {
    let ans = calc.divide({
      x: 4,
      y: 2
    });

    expect(ans).toBe("2");
  });

  it("test add operator", () => {
    let ans = calc.multiply({
      x: 3,
      y: 2
    });

    expect(ans).toBe("6");
  });
});
