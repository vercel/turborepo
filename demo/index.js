import './outer.js';
console.log('Hello World 1');

import fs from 'fs';
import React from 'react';

fs.readFileSync(path.join(__dirname, 'outer.js'));
