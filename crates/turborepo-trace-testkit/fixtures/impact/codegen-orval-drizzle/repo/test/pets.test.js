const { firstPet } = require('../src/pets');

test('pet', () => expect(firstPet().id).toBe(1));
