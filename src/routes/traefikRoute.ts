import { Router } from "express";
import { traefikConfig_get } from "../controllers/traefikControllers";

export const traefikRouter = Router();
traefikRouter.get("/", traefikConfig_get);
