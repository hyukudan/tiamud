import { describe, expect, it } from 'vitest';
import { clampItemQuantity, clampTradeQuantity } from './quantity';

describe('clampItemQuantity', () => {
  it('returns the truncated raw value when within bounds', () => {
    expect(clampItemQuantity(3, 10)).toBe(3);
    expect(clampItemQuantity(3.9, 10)).toBe(3);
  });

  it('clamps below one up to a minimum of one', () => {
    expect(clampItemQuantity(0, 10)).toBe(1);
    expect(clampItemQuantity(-5, 10)).toBe(1);
  });

  it('never exceeds the available maximum', () => {
    expect(clampItemQuantity(100, 4)).toBe(4);
  });

  it('never exceeds the hard cap of 256', () => {
    expect(clampItemQuantity(1000, 1000)).toBe(256);
  });

  it('treats non-finite or NaN raw input as one before clamping', () => {
    expect(clampItemQuantity(Number.NaN, 10)).toBe(1);
    expect(clampItemQuantity(Number.POSITIVE_INFINITY, 10)).toBe(10);
  });

  it('keeps a floor of one even when the maximum is below one', () => {
    expect(clampItemQuantity(5, 0)).toBe(1);
  });
});

describe('clampTradeQuantity', () => {
  it('returns the truncated raw value when within bounds', () => {
    expect(clampTradeQuantity(7, 20)).toBe(7);
    expect(clampTradeQuantity(7.5, 20)).toBe(7);
  });

  it('clamps below one up to a minimum of one', () => {
    expect(clampTradeQuantity(0, 20)).toBe(1);
    expect(clampTradeQuantity(-2, 20)).toBe(1);
  });

  it('never exceeds the available maximum', () => {
    expect(clampTradeQuantity(50, 6)).toBe(6);
  });

  it('never exceeds the hard cap of 128', () => {
    expect(clampTradeQuantity(1000, 1000)).toBe(128);
  });

  it('treats NaN raw input as one before clamping', () => {
    expect(clampTradeQuantity(Number.NaN, 20)).toBe(1);
  });
});
