import { chain } from "mathjs";

export interface InputNumber {
  x: number;
  y: number;
}

export interface CalculatorInterface {
  precision: number;
}

export class Calculator {
  precision: number;

  constructor(param: CalculatorInterface) {
    this.precision = param.precision;
  }

  add(input: InputNumber): string {
    const ans = chain(input.x).add(input.y).done();
    return this.formatPrecision(ans);
  }

  subtract(input: InputNumber): string {
    const ans = chain(input.x).subtract(input.y).done();
    return this.formatPrecision(ans);
  }

  divide(input: InputNumber): string {
    const ans = chain(input.x).divide(input.y).done();
    return this.formatPrecision(ans);
  }

  multiply(input: InputNumber): string {
    const ans = chain(input.x).multiply(input.y).done();
    return this.formatPrecision(ans);
  }

  formatPrecision(input: number): string {
    return input.toPrecision(this.precision);
  }
}
