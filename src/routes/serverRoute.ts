import { Router } from "express";
import {
    server_delete,
    server_post,
    server_put,
    servers_get,
    servers_getByName,
} from "../controllers/serverControllers";

export const serverRouter = Router();
serverRouter.get("/", servers_get);
serverRouter.get("/:name", servers_getByName);
serverRouter.post("/", server_post);
serverRouter.put("/:id", server_put);
serverRouter.delete("/:id", server_delete);
