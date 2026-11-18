import dynamic from 'next/dynamic';
import { PricingTable } from '../../components/pricing-table';

const PlanCalculator = dynamic(() => import('../../components/plan-calculator'));

export default function PricingPage() {
  return (
    <main>
      <h1>Pricing</h1>
      <PricingTable />
      <PlanCalculator />
    </main>
  );
}
