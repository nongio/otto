/**
 * Adds a duration to an ISO 8601 timestamp and returns the result in canonical
 * ISO 8601 form.
 */
export function addMillisecondsToTimestamp(timestamp: string, duration: number): string {
  return new Date(Date.parse(timestamp) + duration).toISOString();
}
