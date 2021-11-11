export default function flatten(list) {
  return list.reduce((flat, toFlatten) => {
    return flat.concat(
      toFlatten.children ? flatten(toFlatten.children) : toFlatten
    )
  }, [])
}
