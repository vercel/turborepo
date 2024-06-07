console.log("Hello");
const value = externalFunction();
const value2 = externalObject.propertyWithGetter;
externalObject.propertyWithSetter = 42;
const shared = { value, value2 };
console.log(shared);

export const a = { shared, a: "aaaaaaaaaaa" };
export const b = { shared, b: "bbbbbbbbbbb" };
