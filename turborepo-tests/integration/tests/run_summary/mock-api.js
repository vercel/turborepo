const http = require("http");
const fs = require("fs");
const url = require("url");
const qs = require("querystring");

let POST_COUNTER = 0;
let PATCH_COUNTER = 0;

const RUN_ID = "1234";

const DEFAULT_PORT = 8000;
const args = process.argv.slice(2);
const port = args.includes("--port")
  ? parseInt(args[args.indexOf("--port") + 1])
  : DEFAULT_PORT;

// Write the process ID to a file, so we can kill it later.
fs.writeFileSync("server.pid", process.pid.toString() + "\n");

class RequestHandler {
  constructor(request, response) {
    this.request = request;
    this.response = response;
  }

  handleRequest() {
    const { method, url: requestUrl } = this.request;

    if (method === "POST") {
      this.handlePostRequest(requestUrl);
      return;
    }

    if (method === "PATCH") {
      this.handlePatchRequest(requestUrl);
      return;
    }

    // send 200 for everything else. add more handling here if needed
    this.sendResponse(200, "");
  }

  handlePostRequest(requestUrl) {
    const filename = `post-${POST_COUNTER}.json`;
    POST_COUNTER++;

    const url = new URL(requestUrl, "http://localhost"); // add a fake hostname
    if (/\/runs/.test(url.pathname)) {
      const response = JSON.stringify({ id: RUN_ID });
      this._recordRequest(filename, response);
      this.sendResponse(200, "application/json", response);
      return;
    }

    this._recordRequest(filename);
    this.sendResponse(200, "");
  }

  handlePatchRequest(requestUrl) {
    const filename = `patch-${PATCH_COUNTER}.json`;
    PATCH_COUNTER++;

    this._recordRequest(filename);
    this.sendResponse(200, "");
  }

  sendResponse(statusCode, contentType, body) {
    this.response.writeHead(statusCode, { "Content-Type": contentType });
    this.response.end(body);
  }

  _recordRequest(filename, response) {
    let body = "";

    this.request.on("data", (chunk) => {
      body += chunk;
    });

    this.request.on("end", () => {
      const requestBody = JSON.parse(body);
      const requestDict = {
        requestUrl: this.request.url,
        requestBody,
      };

      if (response) {
        requestDict.response = JSON.parse(response);
      }

      fs.writeFileSync(filename, JSON.stringify(requestDict));
    });
  }
}

const server = http.createServer((request, response) => {
  const requestHandler = new RequestHandler(request, response);
  requestHandler.handleRequest();
});

server.listen(port, () => {
  // console.log(`Server is listening on port ${port}`);
});
