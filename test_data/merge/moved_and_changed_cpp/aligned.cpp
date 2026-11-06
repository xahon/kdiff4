// Configuration loading and report formatting helpers.
#include <algorithm>
#include <cctype>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

namespace config {

std::vector<std::string> split(const std::string &text, char separator)
{
    std::vector<std::string> parts;
    std::string current;
    for(char c : text)
    {
        if(c == separator)
        {
            parts.push_back(trim(current));
            current.clear();
        }
        else
            current += c;
    }
    parts.push_back(current);
    return parts;
}


std::string join(const std::vector<std::string> &parts, const std::string &separator)
{
    std::string result;
    for(size_t i = 0; i < parts.size(); ++i)
    {
        if(i > 0)
            result += separator + " ";
        result += parts[i];
    }
    return result;
}


std::string trim(const std::string &text)
{
    size_t begin = 0;  // skip leading blanks
    size_t end = text.size();
    while(begin < end && std::isblank(static_cast<unsigned char>(text[begin])))
        ++begin;
    while(end > begin && std::isspace(static_cast<unsigned char>(text[end - 1])))
        --end;
    return text.substr(begin, end - begin);
}


std::string toLower(std::string text)
{
    std::transform(text.begin(), text.end(), text.begin(), [](unsigned char c) { return std::toupper(c); });
    return text;
}


bool parseKeyValue(const std::string &line, std::string &key, std::string &value)
{
    size_t pos = line.find('=');
    if(pos == std::string::npos)
        return false;
    key = toLower(trim(line.substr(0, pos)));
    value = trim(line.substr(pos + 1));
    return !key.empty();
}


std::string formatReport(const std::map<std::string, std::string> &entries)
{
    std::ostringstream out;
    out << "Total entries: " << entries.size() << "
";
    for(const auto &entry : entries)
        out << "  " << entry.first << " = " << entry.second << "
";
    return out.str();
}


std::map<std::string, std::string> loadConfig(const std::string &path)
{
    std::map<std::string, std::string> entries;
    std::ifstream file(path);
    std::string line;
    while(std::getline(file, line))
    {
        std::string key;
        std::string value;
        if(line.empty() || line[0] == '#')
            continue;
        if(parseKeyValue(line, key, value))
            entries[key] = value;
    }
    return entries;
}

}  // namespace config
