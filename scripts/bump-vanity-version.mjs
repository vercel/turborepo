#!/usr/bin/env node
import shelljs from 'shelljs'
import path from 'path'
import fs from 'fs-extra'
import { fileURLToPath } from 'url'
const __dirname = path.dirname(fileURLToPath(import.meta.url))

const file = path.join(__dirname, '../npm/turbo-install-vanity/package.json')

const pkg = fs.readJSONSync(file)

pkg.dependencies['@turborepo/core'] = pkg.version

fs.writeFileSync(file, JSON.stringify(pkg, null, 2))
