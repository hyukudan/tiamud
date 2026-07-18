export const mailVisibleText = (value: string, maximum: number): boolean => {
  const text = value.trim();
  return text === value && text.length > 0 && new TextEncoder().encode(text).length <= maximum &&
    !/[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/.test(text);
};

export const mailDate = (unixSeconds: number): string => new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
}).format(new Date(unixSeconds * 1_000));
