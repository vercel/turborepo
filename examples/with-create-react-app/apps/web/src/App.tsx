import React from "react";
import { Link } from "ui";
import "./App.css";

function App() {
  return (
    <div className="App">
      <header className="App-header">
        <h1 className="header">
          Web
          <div className="Turborepo">Turborepo Example</div>
        </h1>
        <div>
          <Link className="App-link" href="https://turbo.build/repo">
            Turborepo Docs
          </Link>
          <span> | </span>
          <Link className="App-link" href="https://reactjs.org">
            React Docs
          </Link>
        </div>
      </header>
    </div>
  );
}

export default App;
