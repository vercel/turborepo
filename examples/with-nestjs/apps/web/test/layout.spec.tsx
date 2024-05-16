import { render } from '@testing-library/react';

import { metadata, RootLayoutWithoutHtml } from '../app/layout';

describe('Root layout', () => {
  const { container, unmount } = render(<RootLayoutWithoutHtml />, {
    container: document.createElement('html'),
  });

  it('should match the snapshot', () => {
    expect(container).toMatchSnapshot();
  });

  describe('metadata', () => {
    it('should be exported', () => {
      expect(metadata).toBeDefined();
    });

    it('should contain a `title` and `description`', () => {
      expect(metadata).toHaveProperty('title');
      expect(metadata).toHaveProperty('description');
    });
  });

  afterAll(() => {
    unmount();
  });
});
