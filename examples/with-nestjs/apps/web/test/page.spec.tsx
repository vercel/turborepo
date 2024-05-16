import { render } from '@testing-library/react';

import Page from '../app/page';

describe('Root page', () => {
  const { container, unmount } = render(<Page />);

  it('should match the snapshot', () => {
    expect(container).toMatchSnapshot();
  });

  it('should have the correct tree parent', () => {
    expect(container).toBeInstanceOf(HTMLDivElement);
  });

  afterAll(() => {
    unmount();
  });
});
