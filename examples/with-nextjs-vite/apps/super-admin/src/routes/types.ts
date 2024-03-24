export type Auth = {
  login: (username: string) => void;
  logout: () => void;
  status: 'loggedOut' | 'loggedIn';
  username?: string;
};

export const auth: Auth = {
  username: undefined,
  login: (username: string) => {
    console.log('Logging in', username);
    auth.status = 'loggedIn';
    auth.username = username;
  },
  logout: () => {
    console.log('Logging out');
    auth.username = undefined;
    auth.status = 'loggedOut';
  },
  status: 'loggedOut',
};
