import { render } from '@testing-library/react';

import RootPage from '../app/page';

window.fetch = jest.fn().mockImplementation(() =>
  Promise.resolve({
    ok: true,
    json: () => [],
  }),
);

describe('Root page', () => {
  const { container, unmount } = render(
    <RootPage params={{ forTest: true }} />,
  );

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
