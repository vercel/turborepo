import { metadata } from '../app/layout';

describe('Root layout', () => {
  describe('metadata', () => {
    it('should be exported', () => {
      expect(metadata).toBeDefined();
    });

    it('should contain a `title` and `description`', () => {
      expect(metadata).toHaveProperty('title');
      expect(metadata).toHaveProperty('description');
    });
  });
});
