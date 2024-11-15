import { Button } from "./button.tsx";
import foo from "./foo";
import repeat from "repeat-string";

const button = new Button();

button.render();
repeat("foo", 5);
foo();
