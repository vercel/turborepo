import { helloWorld } from '../src/hello-world'

describe('hello-world', () => {
    it('needs tests', () => {
        expect(helloWorld()).toBe("hello world!")
    });
});
