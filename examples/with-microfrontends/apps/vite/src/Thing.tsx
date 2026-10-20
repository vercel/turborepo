import { Link } from "react-router-dom";

function Nested() {
  return (
    <div>
      <h1>Nested Page</h1>
      <p>This is the nested route</p>

      <div style={{ marginTop: "2rem" }}>
        <Link to="/thing">Go to /nested page</Link>
      </div>
    </div>
  );
}

export default Nested;
