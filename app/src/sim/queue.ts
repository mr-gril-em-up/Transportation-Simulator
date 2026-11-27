/** Run async calls one at a time, in call order, regardless of how long each takes.
 *  A failed call does not block the ones after it. */
export function serialQueue(): <T>(call: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(call: () => Promise<T>) => {
    const next = tail.then(call);
    tail = next.catch(() => undefined);
    return next;
  };
}
