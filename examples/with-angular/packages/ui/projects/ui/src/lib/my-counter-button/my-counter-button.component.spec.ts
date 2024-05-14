import { ComponentFixture, TestBed } from '@angular/core/testing';

import { MyCounterButtonComponent } from './my-counter-button.component';

describe('MyCounterButtonComponent', () => {
  let component: MyCounterButtonComponent;
  let fixture: ComponentFixture<MyCounterButtonComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MyCounterButtonComponent]
    }).compileComponents();

    fixture = TestBed.createComponent(MyCounterButtonComponent);
    component = fixture.componentInstance;
    fixture.detectChanges();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });
});
