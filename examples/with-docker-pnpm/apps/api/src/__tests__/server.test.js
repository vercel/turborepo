import supertest from "supertest";
import { createServer } from "../server.js";

describe("server", () => {
  it("health check returns 200", async (done) => {
    await supertest(createServer())
      .get("/healthz")
      .expect(200)
      .then((res) => {
        expect(res.body.ok).toBe(true);
      });

    done();
  });

  it("message endpoint says hello", async (done) => {
    await supertest(createServer())
      .get("/message/jared")
      .expect(200)
      .then((res) => {
        expect(res.body).toEqual({ message: "hello jared" });
      });

    done();
  });
});
