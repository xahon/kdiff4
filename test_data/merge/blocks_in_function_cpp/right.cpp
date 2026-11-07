#include <vector>

int process(std::vector<int> &values)
{
    int total = 0;
    if (values.empty())
    {
        return 0;
    }
    for (int value : values)
    {
        total += value;
    }
    if (total < 0)
    {
        total = 0;
    }
    while (total > 100)
    {
        total -= 50;
    }
    return total;
}

int helper(int x)
{
    return x * 2;
}
