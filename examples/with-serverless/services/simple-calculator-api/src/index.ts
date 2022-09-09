import { Calculator, InputNumber } from "@packages/calculator";
import { Logger } from "@packages/logger";
import { APIGatewayEvent, Context, APIGatewayProxyResult } from "aws-lambda";

interface CalculatorInput {
  operation: "add" | "subtract" | "divide" | "multiply";
  precision: number;
  x: number;
  y: number;
}

interface AnswerData extends CalculatorInput {
  answer: string;
}

async function handler(
  event: APIGatewayEvent,
  _context: Context
): Promise<APIGatewayProxyResult> {
  const logger: Logger = new Logger({
    serviceName: _context.functionName,
  });

  const params: CalculatorInput = JSON.parse(event.body!);

  if (Object.keys(params).length === 0) {
    let error: Error = {
      name: "Input Error",
      message: "Empty Input",
    };
    logger.error("Error:", error);
    throw error;
  };

  logger.info("PROCESSING EVENT:" + JSON.stringify(event, null, 2));

  const calc = new Calculator({
    precision: params.precision,
  });

  let ans: string = "null";
  let inputNumber: InputNumber = {
    x: params.x,
    y: params.y,
  };

  try {
    switch (params.operation) {
      case "add":
        ans = calc.add(inputNumber);
        break;
      case "subtract":
        ans = calc.subtract(inputNumber);
        break;
      case "divide":
        ans = calc.divide(inputNumber);
        break;
      case "multiply":
        ans = calc.multiply(inputNumber);
        break;
      default:
        ans = "Calculation error!";
    }
  } catch (error) {
    logger.error("Calculator Error:", error as Error);
  };

  const answerData: AnswerData = {
    ...params,
    answer: ans,
  };

  logger.info(
    "Calculation completed, answer:" + JSON.stringify(answerData, null, 2)
  );

  return {
    statusCode: 200,
    body: JSON.stringify(answerData, null, 2),
  };
};

exports = {
  handler
};