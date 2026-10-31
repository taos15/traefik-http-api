import cors from "cors";
import express, { Request, Response } from "express";
import { rateLimit } from "express-rate-limit";
import morgan from "morgan";
import path from "path";
import { authRouter } from "./routes/authRoute";
import { dockerContainerRoute } from "./routes/dockerContainerRoute";
import { serverRouter } from "./routes/serverRoute";
import { traefikRouter } from "./routes/traefikRoute";

export const app = express();
const staticFileRateLimiter = rateLimit({
    windowMs: 60 * 1000,
    limit: 100,
    standardHeaders: "draft-7",
    legacyHeaders: false,
});

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));
app.use("/auth", authRouter);
app.use("/api/v1/servers", serverRouter);
app.use("/api/v1/containers", dockerContainerRoute);
app.use("/api/v1/traefikconfig", traefikRouter);
app.use("/", staticFileRateLimiter, express.static(path.join(__dirname, "..", "UI")));

export const domain = process.env.DOMAIN;

export interface Icontainerserver {
    id: string;
    name: string;
    host: string;
    port: number;
    enable: boolean;
}

app.get("/api/ping", (req: Request, res: Response) => {
    res.status(200).send(`server up and running, pong`);
});

// Catch all other requests and serve the index.html file
app.get("*", staticFileRateLimiter, (req: Request, res: Response) => {
    res.sendFile(path.join(__dirname, "..", "UI", "index.html"));
});
