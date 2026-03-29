import { render } from "preact";
import App from "./App";
import { UpdateProvider } from "./contexts/UpdateContext";

render(
  <UpdateProvider>
    <App />
  </UpdateProvider>,
  document.getElementById("root")!,
);
