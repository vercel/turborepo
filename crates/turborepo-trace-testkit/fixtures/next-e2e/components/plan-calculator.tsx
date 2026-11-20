'use client';

import { useState } from 'react';

export default function PlanCalculator() {
  const [seats, setSeats] = useState(1);
  return (
    <label>
      Seats
      <input type="number" value={seats} onChange={(e) => setSeats(Number(e.target.value))} />
      <output>{seats * 20}</output>
    </label>
  );
}
