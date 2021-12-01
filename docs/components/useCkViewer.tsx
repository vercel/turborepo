import * as React from "react";
import useSwr from "swr";
import Cookies from "js-cookie";
import axios from "axios";

const fetcher = (url: string) => axios.get(url).then((res) => res.data);

interface CkUser {
  created_at: string;
  email_address: string;
  fields: Record<string, any>;
  first_name: string;
  id: number;
  state: "active" | "inactive";
}

export const useCkViewer = () => {
  const [ckId, setCkId] = React.useState<string | undefined>();
  React.useEffect(() => {
    const maybeCkId = Cookies.get("ckId");
    if (maybeCkId) {
      setCkId(maybeCkId);
    }
  }, []);
  return useSwr(() => (ckId ? `/api/user/${ckId}` : null), fetcher);
};
