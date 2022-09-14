import { useEffect, useState } from "react";
import { exampleConfigFromShared } from "shared/util";

import "./App.css";

function App() {
  const [data, setData] = useState({});
  useEffect(() => {
    fetch("/api/test")
      .then((res) => res.json())
      .then((res) => setData(res));
  }, []);

  return (
    <div className="App">
      <pre>API: {JSON.stringify(data)}</pre>
      <pre>Shared: {JSON.stringify(exampleConfigFromShared)}</pre>
    </div>
  );
}

export default App;
