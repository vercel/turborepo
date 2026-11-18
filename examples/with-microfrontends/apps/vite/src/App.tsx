import { Link } from "react-router";
import reactLogo from "./assets/react.svg";
import viteLogo from "/vite.svg";
import "./App.css";

function App() {
  return (
    <>
      <div>
        <a href="https://vite.dev" target="_blank">
          <img src={viteLogo} className="logo" alt="Vite logo" />
        </a>
        <a href="https://react.dev" target="_blank">
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
      </div>
      <h1>Vite + React on localhost:3002</h1>
      <p style={{ fontSize: 24 }}>But you're looking at localhost:3024 🤩</p>
      <div style={{ marginTop: "2rem" }}>
        <Link to="/admin/nested">Go to /thing/nested page</Link>
      </div>
      <div style={{ marginTop: "2rem" }}>
        <a href="/">Go to Next.js app</a>
      </div>
    </>
  );
}

export default App;
