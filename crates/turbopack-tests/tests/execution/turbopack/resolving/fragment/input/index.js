import client from "./client#component";
import nofrag from "./nofrag#frag";

it("should resolve to a file with a fragment", () => {
  expect(client).toBe("client#component");
});

it("should resolve to a file without a fragment", () => {
  expect(nofrag).toBe("nofrag");
});
