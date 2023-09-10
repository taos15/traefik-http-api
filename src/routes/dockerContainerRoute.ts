import { Router } from "express";
import {
    dockerContainer_delete,
    dockerContainer_getAll,
    dockerContainer_getByID,
    dockerContainer_getByServer,
    dockerContainer_post,
    dockerContainer_postByID,
    dockerContainer_put,
} from "../controllers/dockerContainerControllers";

export const dockerContainerRoute = Router();
dockerContainerRoute.get("/", dockerContainer_getAll);
dockerContainerRoute.get("/:id", dockerContainer_getByID);
dockerContainerRoute.post("/:id", dockerContainer_postByID);
dockerContainerRoute.get("/server/:name", dockerContainer_getByServer);
dockerContainerRoute.post("/", dockerContainer_post);
dockerContainerRoute.put("/:id", dockerContainer_put);
dockerContainerRoute.delete("/:id", dockerContainer_delete);
