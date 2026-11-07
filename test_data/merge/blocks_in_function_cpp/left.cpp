#include <vector>

int helper(int x)
{
    return x * 2;
}

int process(std::vector<int> &values)
{
    int total = 0;
    if (values.empty())
    {
        return 0;
    }
    for (int value : values)
    {
        total += helper(value);
    }
    if (total < 0)
    {
        total = 0;
    }
    while (total > 100)
    {
        total -= 100;
    }
    return total;
}
