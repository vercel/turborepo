import axios from "axios";

const API_KEY = process.env.CONVERTKIT_API_KEY;
const API_SECRET = process.env.CONVERTKIT_API_SECRET;

const Http = axios.create({
  baseURL: "https://api.convertkit.com/v3",
  headers: {
    "Content-Type": "application/json; charset=utf-8",
  },
});

export function subscribeToForm({
  formId,
  email,
  firstName,
  fields,
}: {
  formId: string;
  email: string;
  firstName: string;
  fields?: Record<string, any>;
}): Promise<Subscriber> {
  return Http(`/forms/${formId}/subscribe`, {
    method: "POST",
    data: { api_key: API_KEY, email, first_name: firstName, fields },
  }).then((res) => res.data.subscription?.subscriber);
}

export function updateSubscriber(
  id: string,
  update: Subscriber
): Promise<unknown> {
  return Http(`/subscribers/${id}`, {
    method: "PUT",
    data: {
      api_secret: API_SECRET,
      ...update,
    },
  }).then((res) => res.data);
}

export interface Subscriber {
  id: number;
  first_name: string;
  email_address: string;
  state: string; // maybe 'active' | 'inactive'
  created_at: string;
  fields: Record<string, any>;
}

export function getSubscriber(id: string): Promise<Subscriber> {
  return Http(`/subscribers/${id}`, {
    method: "GET",
    data: {
      api_secret: API_SECRET,
    },
  }).then((res) => res.data.subscriber);
}
