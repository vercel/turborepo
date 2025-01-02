import { Component } from '@angular/core';

@Component({
  selector: 'lib-my-counter-button',
  standalone: true,
  imports: [],
  templateUrl: './my-counter-button.component.html',
  styleUrl: './my-counter-button.component.css'
})
export class MyCounterButtonComponent {
  count = 0;

  handleClick() {
    this.count += 1;
  }
}
