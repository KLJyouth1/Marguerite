using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace MargueriteDownloader
{
    /// <summary>
    /// A tiny, dependency-free JSON reader. The WPF XAML compiler cannot load
    /// System.Web.Extensions on SDK-style net48 projects, and the downloader only
    /// needs to read a handful of fields from the GitHub release payload, so a
    /// 150-line parser is safer than dragging in a serializer.
    /// </summary>
    public static class Json
    {
        public static object Parse(string text)
        {
            if (text == null)
            {
                throw new ArgumentNullException("text");
            }

            var reader = new Reader(text);
            reader.SkipWhitespace();
            var value = reader.ReadValue();
            reader.SkipWhitespace();

            if (!reader.AtEnd)
            {
                throw new FormatException("Unexpected trailing content at offset " + reader.Offset + ".");
            }

            return value;
        }

        public static Dictionary<string, object> AsObject(object value)
        {
            return value as Dictionary<string, object>;
        }

        public static IList<object> AsArray(object value)
        {
            return value as IList<object>;
        }

        public static string GetString(Dictionary<string, object> source, string key)
        {
            object value;
            return source != null && source.TryGetValue(key, out value) && value != null
                ? Convert.ToString(value, CultureInfo.InvariantCulture)
                : null;
        }

        public static long GetLong(Dictionary<string, object> source, string key)
        {
            object value;
            if (source == null || !source.TryGetValue(key, out value) || value == null)
            {
                return 0L;
            }

            if (value is double)
            {
                return (long)(double)value;
            }

            long parsed;
            return long.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), out parsed) ? parsed : 0L;
        }

        private sealed class Reader
        {
            private readonly string _text;
            private int _index;

            public Reader(string text)
            {
                _text = text;
            }

            public int Offset
            {
                get { return _index; }
            }

            public bool AtEnd
            {
                get { return _index >= _text.Length; }
            }

            public void SkipWhitespace()
            {
                while (_index < _text.Length)
                {
                    var c = _text[_index];
                    if (c == ' ' || c == '\t' || c == '\r' || c == '\n')
                    {
                        _index++;
                        continue;
                    }

                    break;
                }
            }

            public object ReadValue()
            {
                SkipWhitespace();
                if (AtEnd)
                {
                    throw new FormatException("Unexpected end of JSON.");
                }

                var c = _text[_index];
                switch (c)
                {
                    case '{':
                        return ReadObject();
                    case '[':
                        return ReadArray();
                    case '"':
                        return ReadString();
                    case 't':
                        Expect("true");
                        return true;
                    case 'f':
                        Expect("false");
                        return false;
                    case 'n':
                        Expect("null");
                        return null;
                    default:
                        return ReadNumber();
                }
            }

            private Dictionary<string, object> ReadObject()
            {
                var result = new Dictionary<string, object>(StringComparer.Ordinal);
                _index++; // {
                SkipWhitespace();

                if (!AtEnd && _text[_index] == '}')
                {
                    _index++;
                    return result;
                }

                while (true)
                {
                    SkipWhitespace();
                    var key = ReadString();
                    SkipWhitespace();
                    Expect(":");
                    var value = ReadValue();
                    result[key] = value;
                    SkipWhitespace();

                    if (AtEnd)
                    {
                        throw new FormatException("Unterminated object.");
                    }

                    if (_text[_index] == ',')
                    {
                        _index++;
                        continue;
                    }

                    if (_text[_index] == '}')
                    {
                        _index++;
                        return result;
                    }

                    throw new FormatException("Expected ',' or '}' at offset " + _index + ".");
                }
            }

            private List<object> ReadArray()
            {
                var result = new List<object>();
                _index++; // [
                SkipWhitespace();

                if (!AtEnd && _text[_index] == ']')
                {
                    _index++;
                    return result;
                }

                while (true)
                {
                    result.Add(ReadValue());
                    SkipWhitespace();

                    if (AtEnd)
                    {
                        throw new FormatException("Unterminated array.");
                    }

                    if (_text[_index] == ',')
                    {
                        _index++;
                        continue;
                    }

                    if (_text[_index] == ']')
                    {
                        _index++;
                        return result;
                    }

                    throw new FormatException("Expected ',' or ']' at offset " + _index + ".");
                }
            }

            private string ReadString()
            {
                if (AtEnd || _text[_index] != '"')
                {
                    throw new FormatException("Expected a string at offset " + _index + ".");
                }

                _index++;
                var builder = new StringBuilder();

                while (_index < _text.Length)
                {
                    var c = _text[_index++];

                    if (c == '"')
                    {
                        return builder.ToString();
                    }

                    if (c != '\\')
                    {
                        builder.Append(c);
                        continue;
                    }

                    if (_index >= _text.Length)
                    {
                        throw new FormatException("Unterminated escape sequence.");
                    }

                    var escape = _text[_index++];
                    switch (escape)
                    {
                        case '"':
                            builder.Append('"');
                            break;
                        case '\\':
                            builder.Append('\\');
                            break;
                        case '/':
                            builder.Append('/');
                            break;
                        case 'b':
                            builder.Append('\b');
                            break;
                        case 'f':
                            builder.Append('\f');
                            break;
                        case 'n':
                            builder.Append('\n');
                            break;
                        case 'r':
                            builder.Append('\r');
                            break;
                        case 't':
                            builder.Append('\t');
                            break;
                        case 'u':
                            if (_index + 4 > _text.Length)
                            {
                                throw new FormatException("Truncated \\u escape.");
                            }

                            var code = int.Parse(
                                _text.Substring(_index, 4),
                                NumberStyles.HexNumber,
                                CultureInfo.InvariantCulture);
                            builder.Append((char)code);
                            _index += 4;
                            break;
                        default:
                            throw new FormatException("Unsupported escape '\\" + escape + "'.");
                    }
                }

                throw new FormatException("Unterminated string.");
            }

            private double ReadNumber()
            {
                var start = _index;

                if (!AtEnd && (_text[_index] == '-' || _text[_index] == '+'))
                {
                    _index++;
                }

                while (_index < _text.Length)
                {
                    var c = _text[_index];
                    if ((c >= '0' && c <= '9') || c == '.' || c == 'e' || c == 'E' || c == '-' || c == '+')
                    {
                        _index++;
                        continue;
                    }

                    break;
                }

                var slice = _text.Substring(start, _index - start);
                double parsed;
                if (!double.TryParse(slice, NumberStyles.Float, CultureInfo.InvariantCulture, out parsed))
                {
                    throw new FormatException("Invalid number '" + slice + "' at offset " + start + ".");
                }

                return parsed;
            }

            private void Expect(string literal)
            {
                if (_index + literal.Length > _text.Length
                    || string.CompareOrdinal(_text, _index, literal, 0, literal.Length) != 0)
                {
                    throw new FormatException("Expected '" + literal + "' at offset " + _index + ".");
                }

                _index += literal.Length;
            }
        }
    }
}
