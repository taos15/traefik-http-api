import cors from "cors";
import express, { Request, Response } from "express";
import morgan from "morgan";
import path from "path";
import { authRouter } from "./routes/authRoute";
import { dockerContainerRoute } from "./routes/dockerContainerRoute";
import { serverRouter } from "./routes/serverRoute";
import { traefikRouter } from "./routes/traefikRoute";

export const app = express();

app.use(cors());
app.use(express.json());
app.use(morgan("dev"));
app.use("/auth", authRouter);
app.use("/api/v1/servers", serverRouter);
app.use("/api/v1/containers", dockerContainerRoute);
app.use("/api/v1/traefikconfig", traefikRouter);
app.use("/", express.static(path.join(__dirname, "..", "UI")));

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
app.get("*", (req: Request, res: Response) => {
    res.sendFile(path.join(__dirname, "..", "UI", "index.html"));
});
