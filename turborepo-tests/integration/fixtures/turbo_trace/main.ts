import { Button } from "./button.tsx";
import foo from "./foo";

class ButtonWrapper {
  render() {
    Button();
  }
}

const button = new ButtonWrapper();

button.render();
foo();
