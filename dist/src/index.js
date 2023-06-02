import dotenv from "dotenv";
import { app } from "./app.js";
dotenv.config();
const port = 4000;
app.listen(port, () => {
    console.log(`[server]: Server is running at http://localhost:${port}`);
});
//# sourceMappingURL=index.js.map