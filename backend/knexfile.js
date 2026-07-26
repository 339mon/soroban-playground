import path from 'path';
import { fileURLToPath } from 'url';

const _filename = fileURLToPath(import.meta.url);
const _dirname = path.dirname(_filename);

export default {
  development: {
    client: 'sqlite3',
    connection: {
      filename: path.join(_dirname, 'src', 'database', 'database.sqlite'),
    },
    useNullAsDefault: true,
    pool: {
      min: 2,
      max: 10,
    },
    migrations: {
      directory: path.join(_dirname, 'src', 'database', 'migrations'),
    },
  },
  test: {
    client: 'sqlite3',
    connection: {
      filename: ':memory:',
    },
    useNullAsDefault: true,
    migrations: {
      directory: path.join(_dirname, 'src', 'database', 'migrations'),
    },
  },
  production: {
    client: 'sqlite3',
    connection: {
      filename: path.join(_dirname, 'src', 'database', 'database.sqlite'),
    },
    useNullAsDefault: true,
    pool: {
      min: 2,
      max: 10,
    },
    migrations: {
      directory: path.join(_dirname, 'src', 'database', 'migrations'),
    },
  },
};
