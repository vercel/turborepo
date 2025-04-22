export class Counter {
  count = $state(0);

  increment = () => {
    this.count++;
  };

  decrement = () => {
    this.count--;
  };
}
