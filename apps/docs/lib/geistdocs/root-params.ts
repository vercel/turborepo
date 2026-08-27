import { notFound } from "next/navigation";
import * as root from "next/root-params";

export const getRootLang = async () => (await root.lang()) ?? notFound();
